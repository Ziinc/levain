//! gpui-kit's stateless components, as `kit-*` tags. The app reaches them
//! through the language package's generated component functions (from
//! `components/gpui-kit.json`).

use gpui_kit::component::{
    Sizable, Size,
    kbd::Kbd,
    progress::Progress,
    skeleton::Skeleton,
    spinner::Spinner,
    tag::{Tag, TagVariant},
};
use tauri_plugin_gpui::gpui::{self, AnyElement, IntoElement, Keystroke, ParentElement};

use super::{Attr, AttrType, Component, Props, Set};

const CLASS: Attr = Attr {
    name: "class",
    ty: AttrType::String,
    required: false,
    doc: "Layout and style classes, as on a `<div>`.",
};

pub const SET: Set = Set {
    name: "gpui-kit",
    components: &[
        Component {
            tag: "kit-tag",
            doc: "A small status label.",
            attrs: &[
                Attr {
                    name: "variant",
                    ty: AttrType::Enum(&[
                        "primary",
                        "secondary",
                        "danger",
                        "success",
                        "warning",
                        "info",
                    ]),
                    required: false,
                    doc: "Its color. Secondary unless set.",
                },
                Attr {
                    name: "outline",
                    ty: AttrType::Boolean,
                    required: false,
                    doc: "A border without a fill.",
                },
                Attr {
                    name: "pill",
                    ty: AttrType::Boolean,
                    required: false,
                    doc: "Fully rounded ends.",
                },
                Attr {
                    name: "small",
                    ty: AttrType::Boolean,
                    required: false,
                    doc: "The small size.",
                },
                CLASS,
            ],
            events: &[],
            children: true,
            render: tag,
        },
        Component {
            tag: "kit-progress",
            doc: "A horizontal progress bar, named by its `aria-label`.",
            attrs: &[
                Attr {
                    name: "value",
                    ty: AttrType::Number,
                    required: false,
                    doc: "How far along, 0 to 100.",
                },
                Attr {
                    name: "loading",
                    ty: AttrType::Boolean,
                    required: false,
                    doc: "An indeterminate animation in place of `value`.",
                },
                CLASS,
            ],
            events: &[],
            children: false,
            render: progress,
        },
        Component {
            tag: "kit-spinner",
            doc: "A spinning loading indicator.",
            attrs: &[
                Attr {
                    name: "size",
                    ty: AttrType::Enum(&["xsmall", "small", "medium", "large"]),
                    required: false,
                    doc: "Medium unless set.",
                },
                CLASS,
            ],
            events: &[],
            children: false,
            render: spinner,
        },
        Component {
            tag: "kit-skeleton",
            doc: "A pulsing placeholder for content still loading. Size it with `class`.",
            attrs: &[
                Attr {
                    name: "secondary",
                    ty: AttrType::Boolean,
                    required: false,
                    doc: "The secondary color.",
                },
                CLASS,
            ],
            events: &[],
            children: false,
            render: skeleton,
        },
        Component {
            tag: "kit-kbd",
            doc: "A keyboard shortcut, drawn as keycaps.",
            attrs: &[
                Attr {
                    name: "keys",
                    ty: AttrType::String,
                    required: true,
                    doc: "The keystroke, as GPUI writes it (`cmd-k`, `ctrl-shift-p`).",
                },
                Attr {
                    name: "outline",
                    ty: AttrType::Boolean,
                    required: false,
                    doc: "Outlined keycaps.",
                },
                CLASS,
            ],
            events: &[],
            children: false,
            render: kbd,
        },
    ],
};

fn tag(props: Props, _: &mut gpui::Window, _: &mut gpui::App) -> AnyElement {
    let variant = match props.get("variant") {
        Some("primary") => TagVariant::Primary,
        Some("danger") => TagVariant::Danger,
        Some("success") => TagVariant::Success,
        Some("warning") => TagVariant::Warning,
        Some("info") => TagVariant::Info,
        _ => TagVariant::Secondary,
    };
    let mut tag = Tag::new().with_variant(variant);
    if props.flag("outline") {
        tag = tag.outline();
    }
    if props.flag("pill") {
        tag = tag.rounded_full();
    }
    if props.flag("small") {
        tag = tag.with_size(Size::Small);
    }
    let tag = props.styled(tag);
    tag.children(props.children).into_any_element()
}

fn progress(props: Props, _: &mut gpui::Window, _: &mut gpui::App) -> AnyElement {
    let mut bar = Progress::new(gpui::SharedString::from(props.path.to_owned()))
        .value(props.number("value").unwrap_or(0.))
        .loading(props.flag("loading"));
    if let Some(label) = props.get("aria-label") {
        bar = bar.accessibility_label(label.to_owned());
    }
    props.styled(bar).into_any_element()
}

fn spinner(props: Props, _: &mut gpui::Window, _: &mut gpui::App) -> AnyElement {
    let size = match props.get("size") {
        Some("xsmall") => Size::XSmall,
        Some("small") => Size::Small,
        Some("large") => Size::Large,
        _ => Size::Medium,
    };
    // A Spinner takes no style of its own: classes go on a wrapper.
    props
        .styled(gpui::div())
        .child(Spinner::new().with_size(size))
        .into_any_element()
}

fn skeleton(props: Props, _: &mut gpui::Window, _: &mut gpui::App) -> AnyElement {
    let skeleton = if props.flag("secondary") {
        Skeleton::new().secondary()
    } else {
        Skeleton::new()
    };
    props.styled(skeleton).into_any_element()
}

fn kbd(props: Props, _: &mut gpui::Window, _: &mut gpui::App) -> AnyElement {
    let Ok(keystroke) = Keystroke::parse(props.get("keys").unwrap_or("")) else {
        return gpui::Empty.into_any_element();
    };
    let kbd = Kbd::new(keystroke);
    let kbd = if props.flag("outline") {
        kbd.outline()
    } else {
        kbd
    };
    props.styled(kbd).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn is_compiled_in() {
        assert!(
            super::super::sets()
                .iter()
                .any(|set| set.name == "gpui-kit")
        );
        assert_eq!(
            super::super::find("kit-tag").map(|c| c.tag),
            Some("kit-tag")
        );
    }

    #[test]
    fn every_tag_is_prefixed() {
        assert!(SET.components.iter().all(|c| c.tag.starts_with("kit-")));
    }

    // `components/gpui-kit.json` is written from `<host> --manifest gpui-kit`;
    // the language packages generate their component functions from it.
    #[test]
    fn matches_the_committed_manifest() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../components/gpui-kit.json");
        let committed = std::fs::read_to_string(path).unwrap_or_else(|e| {
            panic!("{path}: {e}; regenerate it with `<host> --manifest gpui-kit`")
        });
        let committed: Value = serde_json::from_str(&committed).unwrap();
        assert_eq!(
            super::super::manifest_of("gpui-kit").unwrap(),
            committed,
            "out of date; regenerate it with `<host> --manifest gpui-kit`"
        );
    }
}
