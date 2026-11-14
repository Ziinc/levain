//! Native components registered with the host, and the manifest the app
//! generates function components from (the language package's generated
//! component functions).
//!
//! A component set is a list of [`Component`]s: a tag, the attributes and
//! events it accepts, and a function that renders it from [`Props`]. Sets
//! are compiled in by Cargo features and listed in [`sets`]; the renderer
//! looks a tag up here before its built-in elements. `<host>
//! --manifest` prints every registered component as JSON.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use tauri_plugin_gpui::gpui::{self, AnyElement, Styled};

use crate::style::{self, Theme, Variant};

#[cfg(feature = "components-gpui-kit")]
mod gpui_kit;

/// Bumped whenever the manifest's shape changes.
pub const MANIFEST_VERSION: u32 = 2;

/// What an attribute holds. Values arrive from the app as strings.
#[derive(Clone, Copy, Debug)]
pub enum AttrType {
    /// On when present (and not `"false"`).
    Boolean,
    Number,
    String,
    /// One of the listed strings.
    Enum(&'static [&'static str]),
}

#[derive(Clone, Copy, Debug)]
pub struct Attr {
    pub name: &'static str,
    pub ty: AttrType,
    pub required: bool,
    pub doc: &'static str,
}

/// What a component renders from: its attributes, its rendered children and
/// where it is (for element ids and events).
pub struct Props<'a> {
    pub window: u32,
    pub path: &'a str,
    pub attrs: &'a BTreeMap<String, String>,
    pub theme: &'a Theme,
    pub children: Vec<AnyElement>,
}

impl Props<'_> {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.attrs.get(name).map(String::as_str)
    }

    /// A boolean attribute: on when present, unless it is `"false"`.
    pub fn flag(&self, name: &str) -> bool {
        self.get(name).is_some_and(|value| value != "false")
    }

    pub fn number(&self, name: &str) -> Option<f32> {
        self.get(name)?.parse().ok()
    }

    /// `element` with the component's `class` applied, as on a `<div>`.
    pub fn styled<S: Styled>(&self, element: S) -> S {
        let classes = self.get("class").unwrap_or("");
        style::apply(
            element,
            style::for_variant(classes, Variant::Base),
            self.theme,
        )
    }

    /// Sends the element's `phx-click` (with its `phx-value-*`) when clicked.
    pub fn on_click(
        &self,
    ) -> Option<impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static> {
        crate::view::click_handler(self.window, self.attrs)
    }
}

pub type Render = fn(Props, &mut gpui::Window, &mut gpui::App) -> AnyElement;

pub struct Component {
    pub tag: &'static str,
    pub doc: &'static str,
    pub attrs: &'static [Attr],
    /// The `phx-*` events it sends, without the prefix (`click`).
    pub events: &'static [&'static str],
    /// Whether it renders the children it is given.
    pub children: bool,
    pub render: Render,
}

pub struct Set {
    pub name: &'static str,
    pub components: &'static [Component],
}

/// The sets compiled into this host, each behind its Cargo feature
/// (`components-<name>`).
pub fn sets() -> &'static [Set] {
    &[
        #[cfg(feature = "components-gpui-kit")]
        gpui_kit::SET,
    ]
}

/// The manifest of the compiled-in set called `name`, if there is one.
pub fn manifest_of(name: &str) -> Option<Value> {
    let set = sets().iter().find(|set| set.name == name)?;
    Some(manifest(set, Versions::BUILT))
}

/// The registered component for `tag`, if any.
pub fn find(tag: &str) -> Option<&'static Component> {
    find_in(sets(), tag)
}

fn find_in<'s>(sets: &'s [Set], tag: &str) -> Option<&'s Component> {
    sets.iter()
        .flat_map(|set| set.components)
        .find(|component| component.tag == tag)
}

/// The GPUI and gpui-kit versions a host was built with (from Cargo.lock).
#[derive(Clone, Copy)]
pub struct Versions {
    pub gpui: &'static str,
    pub gpui_kit: &'static str,
}

impl Versions {
    pub const BUILT: Versions = Versions {
        gpui: env!("LEVAIN_GPUI_VERSION"),
        gpui_kit: env!("LEVAIN_GPUI_KIT_VERSION"),
    };
}

/// `set` as the language package's component generator reads it, with the versions it was built
/// against and a hash of all of that.
pub fn manifest(set: &Set, versions: Versions) -> Value {
    let components: Vec<Value> = set
        .components
        .iter()
        .map(|component| {
            json!({
                "tag": component.tag,
                "doc": component.doc,
                "children": component.children,
                "attrs": component.attrs.iter().map(attr).collect::<Vec<_>>(),
                "events": component.events,
            })
        })
        .collect();
    let mut manifest = json!({
        "version": MANIFEST_VERSION,
        "set": set.name,
        "gpui": versions.gpui,
        "gpui_kit": versions.gpui_kit,
        "components": components,
    });
    manifest["hash"] = json!(hash(&manifest));
    manifest
}

/// FNV-1a of the manifest's JSON with its keys sorted, as 16 hex digits.
fn hash(manifest: &Value) -> String {
    let mut sorted = manifest.clone();
    sorted.sort_all_objects();
    let hash = sorted
        .to_string()
        .bytes()
        .fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
        });
    format!("{hash:016x}")
}

/// The hash of each compiled-in set, by name.
pub fn hashes() -> BTreeMap<String, String> {
    hashes_of(sets(), Versions::BUILT)
}

fn hashes_of(sets: &[Set], versions: Versions) -> BTreeMap<String, String> {
    sets.iter()
        .map(|set| {
            let hash = manifest(set, versions)["hash"].as_str().unwrap().to_owned();
            (set.name.to_owned(), hash)
        })
        .collect()
}

/// How the sets the app was compiled against (name to hash) compare with the
/// host's.
#[derive(Debug, Default, PartialEq)]
pub struct Check {
    /// In both, with different hashes: the host refuses these.
    pub mismatched: Vec<String>,
    /// The app's, missing from the host: their tags draw as plain `<div>`s.
    pub missing: Vec<String>,
}

pub fn check(host: &BTreeMap<String, String>, runtime: &BTreeMap<String, String>) -> Check {
    let mut check = Check::default();
    for (set, hash) in runtime {
        match host.get(set) {
            Some(ours) if ours != hash => check.mismatched.push(set.clone()),
            Some(_) => {}
            None => check.missing.push(set.clone()),
        }
    }
    check
}

fn attr(attr: &Attr) -> Value {
    let mut value = json!({
        "name": attr.name,
        "required": attr.required,
        "doc": attr.doc,
    });
    let ty = match attr.ty {
        AttrType::Boolean => "boolean",
        AttrType::Number => "number",
        AttrType::String => "string",
        AttrType::Enum(values) => {
            value["values"] = json!(values);
            "enum"
        }
    };
    value["type"] = json!(ty);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri_plugin_gpui::gpui::{IntoElement, div};

    fn blank(_: Props, _: &mut gpui::Window, _: &mut gpui::App) -> AnyElement {
        div().into_any_element()
    }

    const TEST_VERSIONS: Versions = Versions {
        gpui: "0.0.1",
        gpui_kit: "0.0.2",
    };

    const KIT: &[Set] = &[Set {
        name: "kit",
        components: &[
            Component {
                tag: "kit-badge",
                doc: "A short status label.",
                attrs: &[
                    Attr {
                        name: "variant",
                        ty: AttrType::Enum(&["primary", "danger"]),
                        required: false,
                        doc: "Its color.",
                    },
                    Attr {
                        name: "pill",
                        ty: AttrType::Boolean,
                        required: false,
                        doc: "Fully rounded ends.",
                    },
                ],
                events: &[],
                children: true,
                render: blank,
            },
            Component {
                tag: "kit-progress-bar",
                doc: "How far along something is.",
                attrs: &[
                    Attr {
                        name: "value",
                        ty: AttrType::Number,
                        required: true,
                        doc: "0 to 100.",
                    },
                    Attr {
                        name: "label",
                        ty: AttrType::String,
                        required: false,
                        doc: "Text on the bar.",
                    },
                ],
                events: &["click"],
                children: false,
                render: blank,
            },
        ],
    }];

    #[test]
    fn manifests_only_compiled_in_sets() {
        assert!(manifest_of("no-such-set").is_none());
    }

    #[test]
    fn finds_components_by_tag() {
        assert_eq!(find_in(KIT, "kit-badge").map(|c| c.tag), Some("kit-badge"));
        assert!(find_in(KIT, "div").is_none());
    }

    // The same manifest as the shared fixture (fixtures/components.json),
    // so the two sides agree on its shape.
    #[test]
    fn writes_the_manifest_the_app_reads() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../fixtures/components.json")).unwrap();
        assert_eq!(manifest(&KIT[0], TEST_VERSIONS), fixture);
    }

    #[test]
    fn hashes_change_with_the_components_and_versions() {
        let base = hashes_of(KIT, TEST_VERSIONS)["kit"].clone();
        assert_eq!(base.len(), 16);
        let bumped = Versions {
            gpui: "0.0.9",
            ..TEST_VERSIONS
        };
        assert_ne!(hashes_of(KIT, bumped)["kit"], base);
        let fewer = [Set {
            name: "kit",
            components: &KIT[0].components[..1],
        }];
        assert_ne!(hashes_of(&fewer, TEST_VERSIONS)["kit"], base);
    }

    #[test]
    fn records_the_versions_built_against() {
        assert_eq!(Versions::BUILT.gpui, "0.3.9");
        assert!(!Versions::BUILT.gpui_kit.is_empty());
    }

    #[test]
    fn checks_the_apps_sets_against_the_hosts() {
        let map = |pairs: &[(&str, &str)]| -> BTreeMap<String, String> {
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        };
        let host = map(&[("kit", "aa"), ("other", "bb")]);
        assert_eq!(check(&host, &map(&[("kit", "aa")])), Check::default());
        assert_eq!(
            check(&host, &map(&[("kit", "ff"), ("gone", "cc")])),
            Check {
                mismatched: vec!["kit".into()],
                missing: vec!["gone".into()],
            }
        );
    }

    #[test]
    fn reads_attributes() {
        let attrs: BTreeMap<String, String> = [
            ("pill", ""),
            ("subtle", "false"),
            ("value", "40"),
            ("variant", "danger"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
        let theme = Theme::default();
        let props = Props {
            window: 0,
            path: "0",
            attrs: &attrs,
            theme: &theme,
            children: vec![],
        };
        assert!(props.flag("pill"));
        assert!(!props.flag("subtle"));
        assert!(!props.flag("missing"));
        assert_eq!(props.number("value"), Some(40.0));
        assert_eq!(props.get("variant"), Some("danger"));
    }
}
