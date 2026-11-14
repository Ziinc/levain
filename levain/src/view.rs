//! The JSON view tree the app sends, and its rendering into GPUI elements.

use std::collections::{BTreeMap, HashMap, HashSet};

use gpui_kit::component::{
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
    tooltip::Tooltip,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use tauri_plugin_gpui::gpui::{
    self, AnyElement, AppContext, Entity, InteractiveElement, IntoElement, ParentElement, Role,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Toggled,
    prelude::FluentBuilder,
};

use crate::bridge;
use crate::list::{ListState, Segment};
use crate::style::{self, Theme, Variant};

/// What the app sends a window.
#[derive(Deserialize)]
#[serde(untagged)]
pub enum Message {
    /// Rows of a `<list>`, in reply to the host asking for them.
    Rows {
        rows: Segment,
    },
    /// What changed since the last frame (`patch.rs`), and the lists' rows.
    Patch {
        patch: Vec<crate::patch::Op>,
        #[serde(default)]
        lists: HashMap<String, Segment>,
    },
    Frame(Frame),
}

/// One render from the app: the theme tokens, the root node and, by list id,
/// the rows of each `<list>` on screen.
#[derive(Deserialize, Default)]
pub struct Frame {
    #[serde(default)]
    pub theme: Theme,
    /// Always sent (`null` for no view), so that a malformed patch or rows
    /// message is not taken for an empty frame that blanks the window.
    #[serde(deserialize_with = "Option::deserialize")]
    pub root: Option<Node>,
    #[serde(default)]
    pub lists: HashMap<String, Segment>,
    /// The view's key bindings (`keymap` in the app).
    #[serde(default)]
    pub keymap: Vec<crate::keymap::Binding>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Node {
    Text(String),
    Element {
        #[serde(rename = "t")]
        tag: String,
        #[serde(rename = "a", default)]
        attrs: BTreeMap<String, String>,
        #[serde(rename = "c", default)]
        children: Vec<Node>,
    },
}

/// With the `playwright` feature, registers an element with gpui-kit so
/// Playwright's locators find it by its role and label.
macro_rules! observed {
    // An element that takes focus is observed before it tracks its handle,
    // so Playwright sees the binding.
    ($element:expr, $focus:expr) => {{
        let focus: Option<gpui::FocusHandle> = $focus;
        #[cfg(feature = "playwright")]
        {
            use gpui_kit::base::TestSupportExt;
            let element = $element.test_support();
            match focus {
                Some(handle) => element.track_focus(&handle).into_any_element(),
                None => element.into_any_element(),
            }
        }
        #[cfg(not(feature = "playwright"))]
        {
            match focus {
                Some(handle) => $element.track_focus(&handle).into_any_element(),
                None => $element.into_any_element(),
            }
        }
    }};
    ($element:expr) => {{
        #[cfg(feature = "playwright")]
        {
            use gpui_kit::base::TestSupportExt;
            $element.test_support().into_any_element()
        }
        #[cfg(not(feature = "playwright"))]
        {
            $element.into_any_element()
        }
    }};
}

/// Renders one frame. Holds what rendering needs beyond the node itself:
/// the window's id and theme, the GPUI window and the root's context (to
/// keep inputs' state between frames), and the form being rendered.
struct Renderer<'a, 'c> {
    window_id: u32,
    theme: &'a Theme,
    window: &'a mut gpui::Window,
    cx: &'a mut gpui::Context<'c, Root>,
    inputs: &'a mut HashMap<String, InputSlot>,
    checks: &'a mut HashMap<String, CheckSlot>,
    lists: &'a mut HashMap<String, ListState>,
    focus: &'a mut HashMap<String, gpui::FocusHandle>,
    animations: &'a mut crate::animation::Clock,
    forms: HashMap<String, FormInfo>,
    form: Option<String>,
    seen: HashSet<String>,
    keydowns: Vec<WindowKey>,
}

impl Renderer<'_, '_> {
    fn render(&mut self, node: &Node, path: &str) -> AnyElement {
        match node {
            #[cfg(feature = "playwright")]
            Node::Text(text) => {
                use gpui_kit::base::TestTextExt;
                SharedString::from(text.clone())
                    .test_text(text.clone())
                    .into_any_element()
            }
            #[cfg(not(feature = "playwright"))]
            Node::Text(text) => SharedString::from(text.clone()).into_any_element(),
            Node::Element {
                tag,
                attrs,
                children,
            } if crate::components::find(tag).is_some() => {
                self.component(tag, attrs, children, path)
            }
            Node::Element { tag, attrs, .. } if tag == "list" => self.list(attrs, path),
            Node::Element { tag, attrs, .. } if tag == "line" => render_line(attrs, self.theme),
            Node::Element { tag, attrs, .. } if tag == "icon" => {
                render_icon(attrs, self.theme, path)
            }
            Node::Element {
                tag,
                attrs,
                children,
            } if tag == "canvas" => render_canvas(attrs, children, self.theme),
            Node::Element { tag, attrs, .. }
                if tag == "input" && attrs.get("type").map(String::as_str) == Some("checkbox") =>
            {
                self.checkbox(attrs, path)
            }
            Node::Element { tag, attrs, .. } if tag == "input" || tag == "textarea" => {
                self.input(attrs, path)
            }
            Node::Element {
                tag,
                attrs,
                children,
            } if tag == "form" => {
                self.forms.insert(
                    path.to_owned(),
                    FormInfo {
                        change: attrs.get("phx-change").cloned(),
                        submit: attrs.get("phx-submit").cloned(),
                    },
                );
                let outer = self.form.replace(path.to_owned());
                let element = self.element(tag, attrs, children, path);
                self.form = outer;
                element
            }
            Node::Element {
                tag,
                attrs,
                children,
            } => self.element(tag, attrs, children, path),
        }
    }

    fn element(
        &mut self,
        tag: &str,
        attrs: &BTreeMap<String, String>,
        children: &[Node],
        path: &str,
    ) -> AnyElement {
        let theme = self.theme;
        let window = self.window_id;
        let classes = attrs.get("class").map(String::as_str).unwrap_or("");
        let disabled = attrs.contains_key("disabled");
        if let Some(event) = attrs.get("phx-window-keydown") {
            self.keydowns.push(WindowKey {
                event: event.clone(),
                key: attrs.get("phx-key").cloned(),
                values: values_of(attrs),
            });
        }
        let base = style::apply(
            gpui::div(),
            style::for_variant(classes, Variant::Base),
            theme,
        );
        let base = if disabled {
            style::apply(base, style::for_variant(classes, Variant::Disabled), theme)
        } else {
            base
        };
        let rendered: Vec<AnyElement> = children
            .iter()
            .enumerate()
            .map(|(i, child)| self.render(child, &format!("{path}.{i}")))
            .collect();

        // `animate`, `animate-from`/`animate-to` (`animation.rs`): the style
        // at this moment, over the classes; each frame asks for the next
        // while it runs.
        let mut base = base;
        if let Some(spec) = crate::animation::Spec::from_attrs(attrs, theme) {
            let elapsed = self.animations.elapsed(&crate::animation::key(attrs, path));
            let (style, running) = spec.style_at(elapsed);
            if running {
                self.window.request_animation_frame();
            }
            if let Some(style) = style {
                gpui::Refineable::refine(base.style(), &style);
            }
        }

        let role = role_of(tag, attrs);
        let scroll = |class: &str| classes.split_whitespace().any(|c| c == class);
        let element = base
            .id(SharedString::from(path.to_owned()))
            // Scrolling needs the element's id, so it is not a plain style.
            // `-auto` scrolls too: GPUI shows no scrollbar either way.
            .when(scroll("overflow-scroll") || scroll("overflow-auto"), |el| {
                el.overflow_scroll()
            })
            .when(
                scroll("overflow-x-scroll") || scroll("overflow-x-auto"),
                |el| el.overflow_x_scroll(),
            )
            .when(
                scroll("overflow-y-scroll") || scroll("overflow-y-auto"),
                |el| el.overflow_y_scroll(),
            )
            .children(rendered)
            .when_some(role, |el, role| el.role(role))
            // A label's text is its value in AccessKit, so screen readers (and
            // the AT-SPI end-to-end test) read what a `<text>` shows.
            .when(tag == "text", |el| {
                el.aria_value(SharedString::from(text_of(children)))
            })
            // Headings, links and list items are named by their text unless
            // labelled, as on the web.
            .when(
                !attrs.contains_key("aria-label")
                    && matches!(role, Some(Role::Heading | Role::Link | Role::ListItem)),
                |el| el.aria_label(SharedString::from(text_of(children))),
            )
            .when_some(attrs.get("aria-label"), |el, label| {
                el.aria_label(SharedString::from(label.clone()))
            })
            .when_some(attrs.get("aria-checked"), |el, checked| {
                el.aria_toggled(match checked.as_str() {
                    "true" => Toggled::True,
                    "mixed" => Toggled::Mixed,
                    _ => Toggled::False,
                })
            })
            // `title` is a tooltip on hover, as in a browser.
            .when_some(attrs.get("title").cloned(), |el, title| {
                el.tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
            });

        // `key-context` scopes key bindings to focus inside the element;
        // `tabindex` makes it take focus, in that order on Tab.
        let element = element.when_some(attrs.get("key-context"), |el, context| {
            el.key_context(context.as_str())
        });
        let tab_index = attrs.get("tabindex").and_then(|i| i.parse::<isize>().ok());
        let focus = tab_index.map(|index| {
            self.seen.insert(format!("focus:{path}"));
            let cx = &mut *self.cx;
            self.focus
                .entry(path.to_owned())
                .or_insert_with(|| cx.focus_handle())
                .clone()
                .tab_index(index)
                .tab_stop(true)
        });

        let element = match attrs.get("phx-drag").filter(|_| !disabled) {
            Some(event) => {
                let (up, up_out) = drag_handlers(window, path, event, attrs);
                let key = path.to_owned();
                element
                    // Every element under the pointer hears the press, topmost
                    // first: the topmost draggable claims it.
                    .on_mouse_down(gpui::MouseButton::Left, move |e, _, cx| {
                        DRAGS.with(|d| d.borrow_mut().insert(key.clone(), e.position));
                        cx.stop_propagation();
                    })
                    .on_mouse_up(gpui::MouseButton::Left, up)
                    .on_mouse_up_out(gpui::MouseButton::Left, up_out)
            }
            None => element,
        };

        let on_click: Option<ClickHandler> =
            match (click_handler(window, attrs), self.submits(tag, attrs)) {
                (Some(click), _) => Some(Box::new(click)),
                (None, _) if crate::pick::Pick::from_attrs(window, attrs).is_some() => {
                    let attrs = attrs.clone();
                    Some(Box::new(move |_, _, _| {
                        if let Some(pick) = crate::pick::Pick::from_attrs(window, &attrs) {
                            pick.open();
                        }
                    }))
                }
                (None, Some(form)) => {
                    let root = self.cx.entity().downgrade();
                    Some(Box::new(move |_, _, cx| {
                        let _ = root.update(cx, |root, cx| root.submit(&form, cx));
                    }))
                }
                (None, None) => None,
            };
        let Some(on_click) = on_click.filter(|_| !disabled) else {
            return observed!(element, focus);
        };
        #[cfg(mobile)]
        let element = element.when_some(attrs.get("aria-label"), |el, label| {
            el.child(report_bounds(label.clone()))
        });
        let hover: Vec<String> = style::for_variant(classes, Variant::Hover)
            .map(str::to_owned)
            .collect();
        let active: Vec<String> = style::for_variant(classes, Variant::Active)
            .map(str::to_owned)
            .collect();
        let (hover_theme, active_theme) = (theme.clone(), theme.clone());
        observed!(
            element
                .hover(move |s| style::apply(s, hover.iter(), &hover_theme))
                .active(move |s| style::apply(s, active.iter(), &active_theme))
                .on_click(move |e, w, cx| on_click(e, w, cx)),
            focus
        )
    }

    /// A registered native component (see `components.rs`).
    fn component(
        &mut self,
        tag: &str,
        attrs: &BTreeMap<String, String>,
        children: &[Node],
        path: &str,
    ) -> AnyElement {
        let Some(component) = crate::components::find(tag) else {
            return gpui::Empty.into_any_element();
        };
        let children = if component.children {
            children
                .iter()
                .enumerate()
                .map(|(i, child)| self.render(child, &format!("{path}.{i}")))
                .collect()
        } else {
            vec![]
        };
        let props = crate::components::Props {
            window: self.window_id,
            path,
            attrs,
            theme: self.theme,
            children,
        };
        (component.render)(props, self.window, self.cx)
    }

    /// A virtualized list (GPUI's `uniform_list`) of `rows` rows, drawn as
    /// they scroll into view from the rows the app sends (`list.rs`).
    fn list(&mut self, attrs: &BTreeMap<String, String>, path: &str) -> AnyElement {
        let key = attrs.get("id").cloned().unwrap_or_else(|| path.to_owned());
        self.seen.insert(format!("list:{key}"));
        let state = self.lists.entry(key.clone()).or_default();
        state.count = attrs.get("rows").and_then(|n| n.parse().ok()).unwrap_or(0);
        state.event = attrs.get("phx-rows").cloned();
        state.path = path.to_owned();
        if let Some(height) = attrs.get("row-height").and_then(|h| h.parse().ok()) {
            state.row_height = height;
        }
        let root = self.cx.entity().downgrade();
        let list = gpui::uniform_list(
            SharedString::from(path.to_owned()),
            state.count,
            move |range, window, cx| {
                root.update(cx, |root, cx| root.list_rows(&key, range, window, cx))
                    .unwrap_or_default()
            },
        )
        .with_width_from_item(Some(state.measured()))
        .track_scroll(&state.scroll);
        let classes = attrs.get("class").map(String::as_str).unwrap_or("");
        style::apply(list, style::for_variant(classes, Variant::Base), self.theme)
            .into_any_element()
    }

    /// The form a `<button type="submit">` without its own `phx-click`
    /// submits: the one it is in.
    fn submits(&self, tag: &str, attrs: &BTreeMap<String, String>) -> Option<String> {
        (tag == "button" && attrs.get("type").map(String::as_str) == Some("submit"))
            .then(|| self.form.clone())
            .flatten()
    }

    /// A native text input (gpui-kit's), whose state lives in the root
    /// between frames. Typed text stands until the app renders a different
    /// `value`, as in a browser.
    fn input(&mut self, attrs: &BTreeMap<String, String>, path: &str) -> AnyElement {
        self.seen.insert(path.to_owned());
        let value = attrs.get("value").cloned().unwrap_or_default();
        let placeholder = attrs.get("placeholder").cloned().unwrap_or_default();
        let (window, cx) = (&mut *self.window, &mut *self.cx);

        let slot = self.inputs.entry(path.to_owned()).or_insert_with(|| {
            let masked = attrs.get("type").map(String::as_str) == Some("password");
            let state = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder.clone())
                    .masked(masked)
            });
            state.update(cx, |state, cx| state.set_value(value.clone(), window, cx));
            let key = path.to_owned();
            let subscription = cx.subscribe_in(&state, window, move |root, _, event, _, cx| {
                root.on_input(&key, event, cx)
            });
            InputSlot {
                state,
                rendered: value.clone(),
                placeholder: placeholder.clone(),
                echo: true,
                name: None,
                change: None,
                form: None,
                _subscription: subscription,
            }
        });

        if slot.rendered != value {
            slot.rendered = value.clone();
            if slot.state.read(cx).value() != value.as_str() {
                slot.echo = true;
                slot.state
                    .update(cx, |state, cx| state.set_value(value, window, cx));
            }
        }
        if slot.placeholder != placeholder {
            slot.placeholder = placeholder.clone();
            slot.state.update(cx, |state, cx| {
                state.set_placeholder(placeholder, window, cx)
            });
        }
        slot.name = attrs.get("name").cloned();
        slot.change = attrs.get("phx-change").cloned();
        slot.form = self.form.clone();

        let label = attrs
            .get("aria-label")
            .or(attrs.get("placeholder"))
            .or(attrs.get("name"))
            .cloned()
            .unwrap_or_default();
        let classes = attrs.get("class").map(String::as_str).unwrap_or("");
        style::apply(
            gpui::div(),
            style::for_variant(classes, Variant::Base),
            self.theme,
        )
        .child(
            Input::new(&slot.state)
                .id(SharedString::from(format!("{path}.input")))
                .aria_label(label)
                .disabled(attrs.contains_key("disabled")),
        )
        .into_any_element()
    }

    /// A native checkbox (gpui-kit's). It shows its new state at once;
    /// The app's `checked` wins whenever it changes.
    fn checkbox(&mut self, attrs: &BTreeMap<String, String>, path: &str) -> AnyElement {
        self.seen.insert(path.to_owned());
        let checked = attrs.contains_key("checked");
        let slot = self.checks.entry(path.to_owned()).or_insert(CheckSlot {
            checked,
            rendered: checked,
            ..CheckSlot::default()
        });
        if slot.rendered != checked {
            slot.rendered = checked;
            slot.checked = checked;
        }
        slot.name = attrs.get("name").cloned();
        slot.click = attrs.get("phx-click").cloned();
        slot.values = values_of(attrs);
        slot.form = self.form.clone();

        let root = self.cx.entity().downgrade();
        let key = path.to_owned();
        let label = attrs
            .get("aria-label")
            .or(attrs.get("name"))
            .cloned()
            .unwrap_or_default();
        // gpui-kit components register themselves for Playwright.
        Checkbox::new(SharedString::from(path.to_owned()))
            .checked(slot.checked)
            .accessibility_label(label)
            .on_click(move |checked, _, cx| {
                let _ = root.update(cx, |root, cx| root.on_check(&key, *checked, cx));
            })
            .into_any_element()
    }
}

/// The accessibility role of an element: its `role` attribute, or what its
/// tag means on the web.
fn role_of(tag: &str, attrs: &BTreeMap<String, String>) -> Option<Role> {
    let role = match attrs.get("role") {
        Some(role) => role.as_str(),
        None => match tag {
            "button" => "button",
            "text" | "label" => "label",
            "ul" | "ol" => "list",
            "li" => "listitem",
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
            "a" => "link",
            "img" => "img",
            "nav" => "navigation",
            "form" => "form",
            "main" => "main",
            "header" => "banner",
            "footer" => "contentinfo",
            "aside" => "complementary",
            "section" => "region",
            "article" => "article",
            "p" => "paragraph",
            "dialog" => "dialog",
            "table" => "table",
            "tr" => "row",
            "td" | "th" => "cell",
            "hr" => "separator",
            _ => return None,
        },
    };
    Some(match role {
        "button" => Role::Button,
        "checkbox" => Role::CheckBox,
        "switch" => Role::Switch,
        "radio" => Role::RadioButton,
        "radiogroup" => Role::RadioGroup,
        "textbox" => Role::TextInput,
        "searchbox" => Role::SearchInput,
        "combobox" => Role::ComboBox,
        "listbox" => Role::ListBox,
        "option" => Role::ListBoxOption,
        "label" => Role::Label,
        "list" => Role::List,
        "listitem" => Role::ListItem,
        "heading" => Role::Heading,
        "link" => Role::Link,
        "img" | "image" => Role::Image,
        "navigation" => Role::Navigation,
        "form" => Role::Form,
        "main" => Role::Main,
        "banner" => Role::Banner,
        "contentinfo" => Role::ContentInfo,
        "complementary" => Role::Complementary,
        "region" => Role::Region,
        "article" => Role::Article,
        "paragraph" => Role::Paragraph,
        "dialog" => Role::Dialog,
        "alert" => Role::Alert,
        "status" => Role::Status,
        "tab" => Role::Tab,
        "tablist" => Role::TabList,
        "tabpanel" => Role::TabPanel,
        "menu" => Role::Menu,
        "menuitem" => Role::MenuItem,
        "toolbar" => Role::Toolbar,
        "tooltip" => Role::Tooltip,
        "progressbar" => Role::ProgressIndicator,
        "slider" => Role::Slider,
        "separator" => Role::Splitter,
        "table" => Role::Table,
        "grid" => Role::Grid,
        "row" => Role::Row,
        "cell" | "gridcell" => Role::Cell,
        "tree" => Role::Tree,
        "treeitem" => Role::TreeItem,
        "group" => Role::Group,
        "search" => Role::Search,
        _ => return None,
    })
}

thread_local! {
    /// Where the pointer went down on each `phx-drag` element still held,
    /// by element path.
    static DRAGS: std::cell::RefCell<std::collections::HashMap<String, gpui::Point<gpui::Pixels>>> =
        Default::default();
}

const DRAG_THRESHOLD: f32 = 3.;

type MouseUp = Box<dyn Fn(&gpui::MouseUpEvent, &mut gpui::Window, &mut gpui::App)>;
type ClickHandler = Box<dyn Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App)>;

/// What releasing a `phx-drag` element does, over it or anywhere else:
/// send the event, its `phx-value-*` attributes and how far the pointer
/// moved (`dx`, `dy`) to the view that owns `window`.
fn drag_handlers(
    window: u32,
    path: &str,
    event: &str,
    attrs: &BTreeMap<String, String>,
) -> (MouseUp, MouseUp) {
    let release = {
        let (path, event, values) = (path.to_owned(), event.to_owned(), values_of(attrs));
        move |e: &gpui::MouseUpEvent, _: &mut gpui::Window, _: &mut gpui::App| {
            let Some(start) = DRAGS.with(|d| d.borrow_mut().remove(&path)) else {
                return;
            };
            let (dx, dy) = (
                f32::from(e.position.x - start.x),
                f32::from(e.position.y - start.y),
            );
            // A press released where it started (give or take a shaky hand
            // or finger) is a click, not a drag.
            if dx.abs() < DRAG_THRESHOLD && dy.abs() < DRAG_THRESHOLD {
                return;
            }
            bridge::emit(&crate::events::drag(window, &event, &values, dx, dy));
        }
    };
    let release = std::rc::Rc::new(release);
    let other = release.clone();
    (
        Box::new(move |e, w, cx| release(e, w, cx)),
        Box::new(move |e, w, cx| other(e, w, cx)),
    )
}

/// A lucide icon from gpui-kit's catalog by its kebab-case name
/// (`<icon name="undo-2" class="size-4 text-primary" />`), 16px square
/// unless sized, in its `text-*` colour or else the inherited text colour.
/// An unknown name draws nothing in the same space, and is logged once.
/// `animate="spin"` turns it once every `duration` ms (default 1000).
fn render_icon(attrs: &BTreeMap<String, String>, theme: &Theme, id: &str) -> AnyElement {
    let name = attrs.get("name").map(String::as_str).unwrap_or("");
    let path = icon_path(name);
    if path.is_none() {
        static REPORTED: std::sync::Mutex<Option<HashSet<String>>> = std::sync::Mutex::new(None);
        let mut reported = REPORTED.lock().unwrap();
        if reported.get_or_insert_default().insert(name.to_owned()) {
            crate::log_line(&format!("levain: unknown icon `{name}`"));
        }
    }
    gpui::ViewElement::new(Icon {
        path,
        classes: attrs.get("class").cloned().unwrap_or_default(),
        theme: theme.clone(),
        spin: (attrs.get("animate").map(String::as_str) == Some("spin"))
            .then(|| (SharedString::from(id.to_owned()), duration_of(attrs))),
    })
    .into_any_element()
}

/// The asset path of the icon called `name`, if gpui-kit bundles it.
pub fn icon_path(name: &str) -> Option<SharedString> {
    use gpui::AssetSource;
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    let path = format!("icons/{name}.svg");
    (valid && matches!(gpui_kit::assets::AllAssets.load(&path), Ok(Some(_))))
        .then(|| SharedString::from(path))
}

struct Icon {
    path: Option<SharedString>,
    classes: String,
    theme: Theme,
    spin: Option<(SharedString, std::time::Duration)>,
}

/// An animation's `duration` attribute, in ms (default 1000).
fn duration_of(attrs: &BTreeMap<String, String>) -> std::time::Duration {
    let ms = attrs
        .get("duration")
        .and_then(|d| d.parse().ok())
        .unwrap_or(1000);
    std::time::Duration::from_millis(ms)
}

/// `<canvas>` and its shapes (`paint.rs`), sized by its classes.
fn render_canvas(attrs: &BTreeMap<String, String>, children: &[Node], theme: &Theme) -> AnyElement {
    let shapes = crate::paint::shapes(children, theme);
    let canvas = gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| crate::paint::paint(&shapes, bounds.origin, window),
    );
    let classes = attrs.get("class").map(String::as_str).unwrap_or("");
    style::apply(canvas, style::for_variant(classes, Variant::Base), theme).into_any_element()
}

impl gpui::RenderOnce for Icon {
    fn render(self, window: &mut gpui::Window, _: &mut gpui::App) -> impl IntoElement {
        // Like `currentColor`: the parent's text colour unless a class sets one.
        let inherited = window.text_style().color;
        let svg = gpui::svg()
            .flex_shrink_0()
            .size(gpui::px(16.))
            .text_color(inherited);
        let svg = style::apply(
            svg,
            style::for_variant(&self.classes, Variant::Base),
            &self.theme,
        );
        let svg = match self.path {
            Some(path) => svg.path(path),
            None => svg,
        };
        match self.spin {
            Some((id, period)) => {
                use gpui::AnimationExt;
                svg.with_animation(
                    gpui::ElementId::Name(id),
                    gpui::Animation::new(period).repeat(),
                    |svg, delta| {
                        svg.with_transformation(gpui::Transformation::rotate(gpui::percentage(
                            delta,
                        )))
                    },
                )
                .into_any_element()
            }
            None => svg.into_any_element(),
        }
    }
}

/// A line from `x1`,`y1` to `x2`,`y2`, in pixels from the parent's top left;
/// with `curve`, an S-curve that leaves and enters horizontally.
/// `stroke-<color>` and `stroke-width` style it.
fn render_line(attrs: &BTreeMap<String, String>, theme: &Theme) -> AnyElement {
    let at = |key: &str| {
        attrs
            .get(key)
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(0.)
    };
    let (x1, y1, x2, y2) = (at("x1"), at("y1"), at("x2"), at("y2"));
    let curve = attrs.contains_key("curve");
    let width = attrs
        .get("stroke-width")
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(2.);
    let color = attrs
        .get("class")
        .into_iter()
        .flat_map(|c| c.split_whitespace())
        .find_map(|c| style::color(c.strip_prefix("stroke-")?, theme))
        .unwrap_or_else(gpui::black);
    gpui::canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let origin = bounds.origin;
            let at = |x: f32, y: f32| origin + gpui::point(gpui::px(x), gpui::px(y));
            let mut path = gpui::PathBuilder::stroke(gpui::px(width));
            path.move_to(at(x1, y1));
            if curve {
                // Leaves and enters horizontally, as wires between ports do.
                let bend = ((x2 - x1).abs() / 2.).max(40.);
                path.cubic_bezier_to(at(x2, y2), at(x1 + bend, y1), at(x2 - bend, y2));
            } else {
                path.line_to(at(x2, y2));
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, color);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
    .into_any_element()
}

pub fn values_of(attrs: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    attrs
        .iter()
        .filter_map(|(key, value)| {
            Some((key.strip_prefix("phx-value-")?.to_owned(), value.clone()))
        })
        .collect()
}

/// What clicking an element with `phx-click` does: send the event and its
/// `phx-value-*` attributes to the view that owns `window`.
pub fn click_handler(
    window: u32,
    attrs: &BTreeMap<String, String>,
) -> Option<impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static> {
    let event = attrs.get("phx-click")?.clone();
    let values = values_of(attrs);
    Some(
        move |_: &gpui::ClickEvent, _: &mut gpui::Window, _: &mut gpui::App| {
            bridge::emit(&crate::events::input("click", window, &event, &values));
        },
    )
}

/// Logs where a labelled control is on screen whenever that changes, for the
/// smoke tests, which have no accessibility tree to find controls by: in
/// device pixels on Android (`adb shell input tap`), in points on iOS
/// (`idb ui tap`).
#[cfg(mobile)]
fn report_bounds(label: String) -> impl IntoElement {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static REPORTED: Mutex<Option<HashMap<String, (i32, i32)>>> = Mutex::new(None);
    gpui::canvas(
        move |bounds, window, _| {
            let scale = if cfg!(target_os = "android") {
                window.scale_factor()
            } else {
                1.0
            };
            let center = bounds.center();
            let at = (
                (f32::from(center.x) * scale) as i32,
                (f32::from(center.y) * scale) as i32,
            );
            let mut reported = REPORTED.lock().unwrap();
            if reported.get_or_insert_default().insert(label.clone(), at) != Some(at) {
                crate::log_line(&format!("levain: bounds {label} {} {}", at.0, at.1));
            }
        },
        |_, _, _, _| {},
    )
    // Pinned to the control's corner: unpinned, an absolute child sits where
    // it would have flowed, after the label.
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// The text inside `nodes`, as one string.
pub fn text_of(nodes: &[Node]) -> String {
    let mut parts = Vec::new();
    for node in nodes {
        match node {
            Node::Text(text) => parts.push(text.clone()),
            Node::Element { children, .. } => parts.push(text_of(children)),
        }
    }
    parts.join(" ")
}

/// A text input's state between frames.
struct InputSlot {
    state: Entity<InputState>,
    /// The `value` the app last rendered.
    rendered: String,
    placeholder: String,
    /// Set when the host changed the text, so that change is not sent back.
    echo: bool,
    name: Option<String>,
    change: Option<String>,
    form: Option<String>,
    _subscription: Subscription,
}

/// A checkbox's state between frames.
#[derive(Default)]
struct CheckSlot {
    checked: bool,
    /// The `checked` the app last rendered.
    rendered: bool,
    name: Option<String>,
    click: Option<String>,
    values: BTreeMap<String, String>,
    form: Option<String>,
}

#[derive(Default)]
struct FormInfo {
    change: Option<String>,
    submit: Option<String>,
}

/// The root view of a levain window: whatever the app last rendered, and
/// the state of the native inputs in it.
pub struct Root {
    pub window: u32,
    pub frame: Frame,
    inputs: HashMap<String, InputSlot>,
    checks: HashMap<String, CheckSlot>,
    lists: HashMap<String, ListState>,
    focus: HashMap<String, gpui::FocusHandle>,
    /// When each animated element first rendered.
    animations: crate::animation::Clock,
    /// Set when a patch didn't fit: patches wait for the whole frame.
    desynced: bool,
    /// Lists rendered in other lists' rows since the last render.
    row_lists: HashSet<String>,
    body: Option<gpui::FocusHandle>,
    /// Subscriptions that live as long as the window.
    kept: Vec<Subscription>,
    forms: HashMap<String, FormInfo>,
    keydowns: Vec<WindowKey>,
}

/// A `phx-window-keydown` binding: its event, its `phx-key` filter and its
/// `phx-value-*`.
struct WindowKey {
    event: String,
    key: Option<String>,
    values: BTreeMap<String, String>,
}

/// GPUI's name for a key as the browser's `KeyboardEvent.key` names it
/// (`"Enter"`, `"ArrowUp"`, `" "`); letters stay lowercase whatever Shift
/// does. Lone modifiers have none.
pub fn key_name(key: &str) -> Option<String> {
    Some(
        match key {
            "shift" | "control" | "alt" | "platform" | "function" | "" => return None,
            "enter" => "Enter",
            "escape" => "Escape",
            "backspace" => "Backspace",
            "delete" => "Delete",
            "tab" => "Tab",
            "space" => " ",
            "up" => "ArrowUp",
            "down" => "ArrowDown",
            "left" => "ArrowLeft",
            "right" => "ArrowRight",
            "home" => "Home",
            "end" => "End",
            "pageup" => "PageUp",
            "pagedown" => "PageDown",
            "insert" => "Insert",
            key if key.len() > 1 && key.starts_with('f') && key[1..].parse::<u8>().is_ok() => {
                return Some(key.to_uppercase());
            }
            key => key,
        }
        .to_owned(),
    )
}

impl Root {
    /// Shows `frame`. When its theme tokens changed, gpui-kit's theme takes
    /// them too, so native inputs and checkboxes match the app's colours.
    pub fn show(&mut self, mut frame: Frame, cx: &mut gpui::Context<Self>) {
        if frame.theme != self.frame.theme {
            apply_theme(&frame.theme, cx);
        }
        self.reset_lists(std::mem::take(&mut frame.lists));
        crate::keymap::set(self.window, &frame.keymap, cx);
        self.desynced = false;
        self.frame = frame;
        cx.notify();
    }

    pub fn keep(&mut self, subscription: Subscription) {
        self.kept.push(subscription);
    }

    /// Applies a patch to the frame shown. One that doesn't fit leaves it
    /// part patched and asks the app for the whole frame; the patches before
    /// that frame arrives were diffed against another tree, and are dropped.
    pub fn patch(
        &mut self,
        ops: Vec<crate::patch::Op>,
        lists: HashMap<String, Segment>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.apply_patch(ops, lists) {
            cx.notify();
        }
    }

    /// `patch` without the repaint; false when it was dropped.
    fn apply_patch(&mut self, ops: Vec<crate::patch::Op>, lists: HashMap<String, Segment>) -> bool {
        if self.desynced {
            return false;
        }
        if let Err(error) = crate::patch::apply(&mut self.frame.root, ops) {
            crate::log_line(&format!("levain: patch failed ({error}); resyncing"));
            bridge::emit(&crate::events::resync(self.window));
            self.desynced = true;
        }
        self.reset_lists(lists);
        true
    }

    fn reset_lists(&mut self, mut lists: HashMap<String, Segment>) {
        for (key, list) in &mut self.lists {
            list.reset(lists.remove(key));
        }
    }

    /// Rows the app sent for one of the window's lists.
    pub fn rows(&mut self, segment: Segment, cx: &mut gpui::Context<Self>) {
        if let Some(list) = self.lists.get_mut(&segment.id) {
            list.insert(segment);
            cx.notify();
        }
    }

    /// The rows of list `key` in `range`, as `uniform_list` draws them. Rows
    /// not here yet are blank until the app sends them, which this asks for.
    fn list_rows(
        &mut self,
        key: &str,
        range: std::ops::Range<usize>,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(mut list) = self.lists.remove(key) else {
            return Vec::new();
        };
        if let (Some(want), Some(event)) = (list.wanted(range.clone()), &list.event) {
            bridge::emit(&crate::events::rows(
                self.window,
                event,
                key,
                want.rows.start,
                want.rows.end,
                [want.visible.start, want.visible.end],
            ));
        }
        let frame = std::mem::take(&mut self.frame);
        let mut inputs = std::mem::take(&mut self.inputs);
        let mut checks = std::mem::take(&mut self.checks);
        let mut lists = std::mem::take(&mut self.lists);
        let mut focus = std::mem::take(&mut self.focus);
        let mut animations = std::mem::take(&mut self.animations);
        let rows = {
            let mut renderer = Renderer {
                window_id: self.window,
                theme: &frame.theme,
                window,
                cx,
                inputs: &mut inputs,
                checks: &mut checks,
                lists: &mut lists,
                focus: &mut focus,
                animations: &mut animations,
                forms: HashMap::new(),
                form: None,
                seen: HashSet::new(),
                keydowns: Vec::new(),
            };
            let rows = range
                .map(|i| match list.rows.get(&i) {
                    Some(node) => renderer.render(node, &format!("{}.rows.{i}", list.path)),
                    None => gpui::div().h(gpui::px(list.row_height)).into_any_element(),
                })
                .collect();
            // Lists in these rows keep their state through the next render.
            let nested = renderer.seen.into_iter().filter(|k| k.starts_with("list:"));
            self.row_lists.extend(nested);
            rows
        };
        lists.insert(key.to_owned(), list);
        self.lists = lists;
        self.focus = focus;
        self.animations = animations;
        self.inputs = inputs;
        self.checks = checks;
        self.frame = frame;
        rows
    }

    pub fn new(window: u32, frame: Frame) -> Self {
        Self {
            window,
            frame,
            inputs: HashMap::new(),
            checks: HashMap::new(),
            lists: HashMap::new(),
            focus: HashMap::new(),
            animations: crate::animation::Clock::default(),
            desynced: false,
            row_lists: HashSet::new(),
            body: None,
            kept: Vec::new(),
            forms: HashMap::new(),
            keydowns: Vec::new(),
        }
    }

    /// Sends each `phx-window-keydown` binding the keystroke matches. A
    /// keystroke something already handled (a text input's own Ctrl+Z, Enter
    /// in a form) sends nothing, and while a text input has focus only
    /// chords with Ctrl or Meta do, so typing never fires shortcuts.
    pub fn on_keystroke(&self, event: &gpui::KeystrokeEvent) {
        if event.action.is_some() {
            return;
        }
        let Some(key) = key_name(&event.keystroke.key) else {
            return;
        };
        let mods = &event.keystroke.modifiers;
        let typing = event.context_stack.iter().any(|c| c.contains("Input"));
        if typing && !(mods.control || mods.platform) {
            return;
        }
        for binding in &self.keydowns {
            if binding
                .key
                .as_ref()
                .is_some_and(|k| !k.eq_ignore_ascii_case(&key))
            {
                continue;
            }
            let mut values: Map<String, Value> = binding
                .values
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            values.insert("key".into(), Value::String(key.clone()));
            values.insert("ctrl".into(), Value::Bool(mods.control));
            values.insert("shift".into(), Value::Bool(mods.shift));
            values.insert("alt".into(), Value::Bool(mods.alt));
            values.insert("meta".into(), Value::Bool(mods.platform));
            self.emit("keydown", &binding.event, values);
        }
    }

    fn emit(&self, kind: &str, event: &str, values: Map<String, Value>) {
        bridge::emit(&crate::events::input(kind, self.window, event, values));
    }

    /// Every named value in `form`: each text input's text, and "on" for
    /// each checked checkbox, as a browser submits them.
    fn form_values(&self, form: &str, cx: &gpui::App) -> Map<String, Value> {
        let mut values = Map::new();
        for slot in self.inputs.values() {
            if let (Some(name), Some(f)) = (&slot.name, &slot.form)
                && f == form
            {
                let text = slot.state.read(cx).value().to_string();
                values.insert(name.clone(), Value::String(text));
            }
        }
        self.add_checked(form, &mut values);
        values
    }

    /// "on" for each checked, named checkbox in `form`.
    fn add_checked(&self, form: &str, values: &mut Map<String, Value>) {
        for slot in self.checks.values() {
            if let (Some(name), Some(f), true) = (&slot.name, &slot.form, slot.checked)
                && f == form
            {
                values.insert(name.clone(), Value::String("on".into()));
            }
        }
    }

    fn submit(&self, form: &str, cx: &gpui::App) {
        if let Some(event) = self.forms.get(form).and_then(|f| f.submit.as_ref()) {
            self.emit("submit", event, self.form_values(form, cx));
        }
    }

    /// Sends the input's `phx-change`, or its form's, with `_target`.
    fn changed(
        &self,
        form: Option<&String>,
        own: Option<&String>,
        name: &str,
        value: Value,
        cx: &gpui::App,
    ) {
        let form_change = form
            .and_then(|f| self.forms.get(f))
            .and_then(|f| f.change.as_ref());
        let Some(event) = own.or(form_change) else {
            return;
        };
        let mut values = match form {
            Some(form) => self.form_values(form, cx),
            None => Map::from_iter([(name.to_owned(), value)]),
        };
        values.insert("_target".into(), Value::String(name.to_owned()));
        self.emit("change", event, values);
    }

    fn on_input(&mut self, path: &str, event: &InputEvent, cx: &mut gpui::Context<Self>) {
        let Some(slot) = self.inputs.get_mut(path) else {
            return;
        };
        match event {
            InputEvent::Change => {
                let text = slot.state.read(cx).value().to_string();
                if std::mem::take(&mut slot.echo) && text == slot.rendered {
                    return;
                }
                let (form, own) = (slot.form.clone(), slot.change.clone());
                let name = slot.name.clone().unwrap_or_else(|| "value".into());
                self.changed(form.as_ref(), own.as_ref(), &name, Value::String(text), cx);
            }
            InputEvent::PressEnter { .. } => {
                if let Some(form) = slot.form.clone() {
                    self.submit(&form, cx);
                }
            }
            _ => {}
        }
    }

    fn on_check(&mut self, path: &str, checked: bool, cx: &mut gpui::Context<Self>) {
        let Some(slot) = self.checks.get_mut(path) else {
            return;
        };
        slot.checked = checked;
        cx.notify();
        let (click, form, name) = (slot.click.clone(), slot.form.clone(), slot.name.clone());
        if let Some(event) = click {
            let mut values: Map<String, Value> = slot
                .values
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            values.insert("checked".into(), Value::Bool(checked));
            self.emit("click", &event, values);
        }
        if let (Some(_), Some(name)) = (&form, name) {
            let value = if checked {
                Value::String("on".into())
            } else {
                Value::Null
            };
            self.changed(form.as_ref(), None, &name, value, cx);
        }
    }
}

/// Gives gpui-kit's theme the app's tokens, as ixtable brands it: into both
/// theme configs, then re-resolved with `Theme::change`, so components that
/// read derived colours (a checkbox's fill) take them too. Tokens the app
/// does not define keep gpui-kit's own colours.
fn apply_theme(tokens: &Theme, cx: &mut gpui::App) {
    use gpui_kit::component::{Theme as KitTheme, ThemeConfig};
    use std::rc::Rc;

    let brand = |config: &ThemeConfig| {
        let mut config = config.clone();
        let colors = &mut config.colors;
        let token = |name: &str| tokens.get(name).map(|v| SharedString::from(v.clone()));
        let set = |field: &mut Option<SharedString>, names: &[&str]| {
            if let Some(value) = names.iter().find_map(|name| token(name)) {
                *field = Some(value);
            }
        };
        set(&mut colors.background, &["background"]);
        set(&mut colors.popover, &["background"]);
        set(&mut colors.foreground, &["foreground"]);
        set(&mut colors.popover_foreground, &["foreground"]);
        set(&mut colors.caret, &["foreground"]);
        set(&mut colors.muted, &["muted"]);
        set(&mut colors.muted_foreground, &["muted-foreground"]);
        set(&mut colors.border, &["border"]);
        set(&mut colors.input, &["input", "border"]);
        set(&mut colors.primary, &["primary"]);
        set(&mut colors.primary_hover, &["primary-hover", "primary"]);
        set(&mut colors.primary_active, &["primary-active", "primary"]);
        set(&mut colors.primary_foreground, &["primary-foreground"]);
        set(&mut colors.ring, &["ring", "primary"]);
        set(&mut colors.list_active_border, &["primary"]);
        set(&mut colors.secondary, &["secondary"]);
        set(&mut colors.secondary_foreground, &["secondary-foreground"]);
        set(&mut colors.accent, &["accent"]);
        set(&mut colors.accent_foreground, &["accent-foreground"]);
        set(&mut colors.link, &["link"]);
        set(&mut colors.success, &["success"]);
        set(&mut colors.warning, &["warning"]);
        set(&mut colors.danger, &["danger"]);
        Rc::new(config)
    };

    KitTheme::update(cx, |theme| {
        theme.light_theme = brand(&theme.light_theme);
        theme.dark_theme = brand(&theme.dark_theme);
    });
    let mode = KitTheme::global(cx).mode;
    KitTheme::change(mode, None, cx);
    cx.refresh_windows();
}

impl gpui::Render for Root {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let frame = std::mem::take(&mut self.frame);
        let mut inputs = std::mem::take(&mut self.inputs);
        let mut checks = std::mem::take(&mut self.checks);
        let mut lists = std::mem::take(&mut self.lists);
        let mut focus = std::mem::take(&mut self.focus);
        let mut animations = std::mem::take(&mut self.animations);
        let (content, forms, seen) = {
            let mut renderer = Renderer {
                window_id: self.window,
                theme: &frame.theme,
                window,
                cx,
                inputs: &mut inputs,
                checks: &mut checks,
                lists: &mut lists,
                focus: &mut focus,
                animations: &mut animations,
                forms: HashMap::new(),
                form: None,
                seen: HashSet::new(),
                keydowns: Vec::new(),
            };
            let content = frame
                .root
                .as_ref()
                .map(|node| renderer.render(node, "root"));
            self.keydowns = renderer.keydowns;
            (content, renderer.forms, renderer.seen)
        };
        // Inputs and lists no longer rendered drop their state. Those in a
        // list's rows are drawn later, as the list lays out.
        let kept = |path: &String| seen.contains(path) || path.contains(".rows.");
        inputs.retain(|path, _| kept(path));
        checks.retain(|path, _| kept(path));
        let row_lists = std::mem::take(&mut self.row_lists);
        lists.retain(|key, _| {
            let key = format!("list:{key}");
            seen.contains(&key) || row_lists.contains(&key)
        });
        focus.retain(|path, _| path.contains(".rows.") || seen.contains(&format!("focus:{path}")));
        self.inputs = inputs;
        self.checks = checks;
        // Start times outlive their element for a while: rows scroll back.
        animations.prune();
        self.lists = lists;
        self.focus = focus;
        self.animations = animations;
        self.forms = forms;
        self.frame = frame;
        // The window's content holds focus when nothing in it does, as a
        // browser's body does, so bindings (Tab, keymaps without a
        // context) reach it before anything is clicked.
        let body = self
            .body
            .get_or_insert_with(|| cx.focus_handle().tab_stop(false))
            .clone();
        if window.focused(cx).is_none() {
            window.focus(&body, cx);
        }
        // The view's actions (`keymap.rs`): the body is on every dispatch
        // path, since it holds focus when nothing inside it does.
        let window_id = self.window;
        let body_div = gpui::div()
            .id("body")
            .size_full()
            .key_context(crate::keymap::window_context(window_id).as_str())
            .on_action(move |action: &crate::keymap::RuntimeAction, _, _| {
                bridge::emit(&crate::events::action(window_id, action.name.as_ref()));
            })
            .children(content);
        observed!(body_div, Some(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_frames_and_rows() {
        let frame = r#"{"theme": {}, "root": {"t": "list", "a": {"id": "jobs"}},
            "lists": {"jobs": {"start": 4, "rows": ["a", "b"]}},
            "keymap": [{"keys": "ctrl-k", "action": "open", "context": "Jobs"}]}"#;
        let Ok(Message::Frame(frame)) = serde_json::from_str(frame) else {
            panic!("not a frame");
        };
        assert_eq!(frame.lists["jobs"].start, 4);
        assert_eq!(frame.lists["jobs"].rows.len(), 2);
        assert_eq!(frame.keymap[0].context.as_deref(), Some("Jobs"));

        let rows = r#"{"rows": {"id": "jobs", "start": 9, "rows": [{"t": "text", "c": ["x"]}]}}"#;
        let Ok(Message::Rows { rows }) = serde_json::from_str(rows) else {
            panic!("not rows");
        };
        assert_eq!(
            (rows.id.as_str(), rows.start, rows.rows.len()),
            ("jobs", 9, 1)
        );

        let patch = r#"{"patch": [{"op": "replace", "at": [0], "node": "2"}], "lists": {}}"#;
        let Ok(Message::Patch { patch, .. }) = serde_json::from_str(patch) else {
            panic!("not a patch");
        };
        assert_eq!(patch.len(), 1);
    }

    #[test]
    fn keys_take_the_browsers_names() {
        assert_eq!(key_name("z").as_deref(), Some("z"));
        assert_eq!(key_name("enter").as_deref(), Some("Enter"));
        assert_eq!(key_name("up").as_deref(), Some("ArrowUp"));
        assert_eq!(key_name("space").as_deref(), Some(" "));
        assert_eq!(key_name("f5").as_deref(), Some("F5"));
        assert_eq!(key_name("="), Some("=".into()));
        assert_eq!(key_name("shift"), None);
    }

    #[test]
    fn icons_resolve_by_kebab_case_name() {
        assert_eq!(icon_path("undo-2").as_deref(), Some("icons/undo-2.svg"));
        assert!(icon_path("zoom-in").is_some());
        assert!(icon_path("no-such-icon").is_none());
        assert!(icon_path("../Cargo").is_none());
        assert!(icon_path("").is_none());
    }

    #[test]
    fn icons_are_embedded_in_every_build() {
        // A debug build that reads them from the build machine's disk has
        // none on a phone.
        let icon = gpui_kit::assets::AllAssets::get("icons/save.svg").expect("save icon");
        assert!(matches!(icon.data, std::borrow::Cow::Borrowed(_)));
    }

    fn el(tag: &str, attrs: &[(&str, &str)], children: Vec<Node>) -> Node {
        Node::Element {
            tag: tag.into(),
            attrs: attrs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            children,
        }
    }

    fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    pub(super) fn ops(json: &str) -> Vec<crate::patch::Op> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn nodes_read_the_short_keys() {
        let node: Node = serde_json::from_str(
            r#"{"t": "div", "a": {"class": "p-2"}, "c": ["hi", {"t": "span"}]}"#,
        )
        .unwrap();
        assert_eq!(
            node,
            el(
                "div",
                &[("class", "p-2")],
                vec![Node::Text("hi".into()), el("span", &[], vec![])]
            )
        );
    }

    #[test]
    fn nodes_need_a_tag_and_string_attributes() {
        assert_eq!(
            serde_json::from_str::<Node>(r#""plain""#).unwrap(),
            Node::Text("plain".into())
        );
        // The long names are not the wire format.
        assert!(serde_json::from_str::<Node>(r#"{"tag": "div"}"#).is_err());
        assert!(serde_json::from_str::<Node>(r#"{"a": {"x": "1"}}"#).is_err());
        // Attribute values are strings, as the app stringifies them.
        assert!(serde_json::from_str::<Node>(r#"{"t": "div", "a": {"rows": 3}}"#).is_err());
        assert!(serde_json::from_str::<Node>("42").is_err());
    }

    #[test]
    fn frames_default_what_they_omit() {
        let Ok(Message::Frame(frame)) = serde_json::from_str(r#"{"root": "x"}"#) else {
            panic!("not a frame");
        };
        assert_eq!(frame.root, Some(Node::Text("x".into())));
        assert!(frame.theme.is_empty() && frame.lists.is_empty() && frame.keymap.is_empty());

        let Ok(Message::Frame(frame)) =
            serde_json::from_str(r##"{"theme": {"primary": "#2563eb"}, "root": null}"##)
        else {
            panic!("not a frame");
        };
        assert_eq!(frame.root, None);
        assert_eq!(frame.theme["primary"], "#2563eb");
    }

    #[test]
    fn keymap_bindings_may_omit_their_context() {
        let Ok(Message::Frame(frame)) = serde_json::from_str(
            r#"{"root": null, "keymap": [{"keys": "escape", "action": "close"}]}"#,
        ) else {
            panic!("not a frame");
        };
        assert_eq!(frame.keymap[0].context, None);
        assert_eq!(frame.keymap[0].action, "close");
    }

    #[test]
    fn patches_carry_their_lists_and_default_to_none() {
        let Ok(Message::Patch { patch, lists }) = serde_json::from_str(
            r#"{"patch": [{"op": "attrs", "at": [], "a": {"x": "1"}}],
                "lists": {"jobs": {"start": 2, "rows": ["r"]}}}"#,
        ) else {
            panic!("not a patch");
        };
        assert_eq!(patch.len(), 1);
        assert_eq!(lists["jobs"].start, 2);
        let Ok(Message::Patch { lists, .. }) = serde_json::from_str(r#"{"patch": []}"#) else {
            panic!("not a patch");
        };
        assert!(lists.is_empty());
    }

    #[test]
    // A message that is none of the three must not blank the window: it is
    // reported as invalid (`windows::deliver` shows a notice).
    fn malformed_messages_are_errors_not_empty_frames() {
        for json in [
            r#"{"patch": [{"op": "nope", "at": []}]}"#,
            r#"{"rows": {"id": "a"}}"#,
            r#"[]"#,
            r#""text""#,
            "not json",
        ] {
            assert!(serde_json::from_str::<Message>(json).is_err(), "{json}");
        }
    }

    #[test]
    fn values_come_from_phx_value_attributes() {
        let attrs = attrs(&[
            ("phx-click", "go"),
            ("phx-value-id", "7"),
            ("phx-value-kind", "a-b"),
            ("value", "not this"),
            ("phx-values", "nor this"),
        ]);
        assert_eq!(
            values_of(&attrs),
            self::attrs(&[("id", "7"), ("kind", "a-b")])
        );
        assert!(values_of(&BTreeMap::new()).is_empty());
    }

    #[test]
    fn text_is_gathered_depth_first_and_space_joined() {
        let nodes = [
            Node::Text("a".into()),
            el(
                "b",
                &[],
                vec![
                    Node::Text("b".into()),
                    el("i", &[], vec![Node::Text("c".into())]),
                ],
            ),
        ];
        assert_eq!(text_of(&nodes), "a b c");
        assert_eq!(text_of(&[]), "");
    }

    #[test]
    fn roles_follow_the_tag_unless_given() {
        assert_eq!(role_of("button", &attrs(&[])), Some(Role::Button));
        assert_eq!(role_of("h3", &attrs(&[])), Some(Role::Heading));
        assert_eq!(role_of("a", &attrs(&[])), Some(Role::Link));
        assert_eq!(role_of("header", &attrs(&[])), Some(Role::Banner));
        assert_eq!(role_of("div", &attrs(&[])), None);
        assert_eq!(
            role_of("div", &attrs(&[("role", "checkbox")])),
            Some(Role::CheckBox)
        );
        // An explicit role wins over the tag's.
        assert_eq!(
            role_of("button", &attrs(&[("role", "tab")])),
            Some(Role::Tab)
        );
        assert_eq!(
            role_of("div", &attrs(&[("role", "image")])),
            Some(Role::Image)
        );
        // A role GPUI has no name for gives none, even on a button.
        assert_eq!(role_of("button", &attrs(&[("role", "bogus")])), None);
    }

    #[test]
    fn durations_default_to_a_second() {
        let ms = |a: &[(&str, &str)]| duration_of(&attrs(a)).as_millis();
        assert_eq!(ms(&[]), 1000);
        assert_eq!(ms(&[("duration", "250")]), 250);
        assert_eq!(ms(&[("duration", "slow")]), 1000);
        assert_eq!(ms(&[("duration", "-5")]), 1000);
    }

    #[test]
    fn function_keys_and_modifiers() {
        assert_eq!(key_name("f12").as_deref(), Some("F12"));
        // Not function keys: letters, or numbers that aren't a key.
        assert_eq!(key_name("f").as_deref(), Some("f"));
        assert_eq!(key_name("foo").as_deref(), Some("foo"));
        assert_eq!(key_name("f999").as_deref(), Some("f999"));
        for modifier in ["control", "alt", "platform", "function", ""] {
            assert_eq!(key_name(modifier), None, "{modifier}");
        }
        for (gpui_key, browser_key) in [
            ("escape", "Escape"),
            ("backspace", "Backspace"),
            ("delete", "Delete"),
            ("tab", "Tab"),
            ("down", "ArrowDown"),
            ("left", "ArrowLeft"),
            ("right", "ArrowRight"),
            ("home", "Home"),
            ("end", "End"),
            ("pageup", "PageUp"),
            ("pagedown", "PageDown"),
            ("insert", "Insert"),
        ] {
            assert_eq!(key_name(gpui_key).as_deref(), Some(browser_key));
        }
    }

    fn root() -> Root {
        Root::new(
            3,
            Frame {
                root: Some(el(
                    "div",
                    &[],
                    vec![Node::Text("a".into()), Node::Text("b".into())],
                )),
                ..Frame::default()
            },
        )
    }

    #[test]
    fn a_patch_that_fits_changes_the_frame_and_sends_nothing() {
        let h = crate::bridge::testing::harness();
        let mut root = root();
        let patch = ops(r#"[{"op": "replace", "at": [1], "node": "B"},
                {"op": "attrs", "at": [], "a": {"class": "x"}}]"#);
        assert!(root.apply_patch(patch, HashMap::new()));
        assert_eq!(
            root.frame.root,
            Some(el(
                "div",
                &[("class", "x")],
                vec![Node::Text("a".into()), Node::Text("B".into())]
            ))
        );
        assert!(!root.desynced);
        assert!(h.events().is_empty());
    }

    #[test]
    fn a_patch_that_does_not_fit_asks_for_the_whole_frame_once() {
        let h = crate::bridge::testing::harness();
        let mut root = root();
        let bad = ops(r#"[{"op": "replace", "at": [0], "node": "A"},
                {"op": "replace", "at": [9], "node": "?"}]"#);
        assert!(root.apply_patch(bad, HashMap::new()));
        assert!(root.desynced);
        assert_eq!(
            h.events(),
            [serde_json::json!({"type": "resync", "window": 3})]
        );
        // The tree is left part patched.
        let Some(Node::Element { children, .. }) = &root.frame.root else {
            panic!("no root");
        };
        assert_eq!(children[0], Node::Text("A".into()));

        // Until the frame arrives, patches (made against the lost tree) are
        // dropped, and the app is not asked again.
        let before = root.frame.root.clone();
        let next = ops(r#"[{"op": "replace", "at": [1], "node": "late"}]"#);
        assert!(!root.apply_patch(next, HashMap::new()));
        assert_eq!(root.frame.root, before);
        assert!(h.events().is_empty());
    }

    #[test]
    fn patches_replace_every_lists_rows() {
        let _h = crate::bridge::testing::harness();
        let mut root = root();
        let segment = |start: usize, rows: &[&str]| Segment {
            id: String::new(),
            start,
            rows: rows.iter().map(|r| Node::Text(r.to_string())).collect(),
        };
        for key in ["jobs", "logs"] {
            let mut list = ListState::default();
            list.count = 10;
            list.insert(segment(0, &["old", "old"]));
            root.lists.insert(key.into(), list);
        }
        let lists = HashMap::from([("jobs".to_string(), segment(5, &["new"]))]);
        assert!(root.apply_patch(Vec::new(), lists));
        // `jobs` has the rows the patch brought, and none of its old ones;
        // `logs` got none, so it is emptied to be asked for again.
        let jobs = &root.lists["jobs"];
        assert_eq!(jobs.rows.keys().copied().collect::<Vec<_>>(), [5]);
        assert!(root.lists["logs"].rows.is_empty());
    }

    #[test]
    fn forms_collect_their_checked_boxes() {
        let mut root = root();
        let slot = |name: Option<&str>, form: Option<&str>, checked: bool| CheckSlot {
            checked,
            name: name.map(Into::into),
            form: form.map(Into::into),
            ..CheckSlot::default()
        };
        root.checks
            .insert("a".into(), slot(Some("agree"), Some("root.0"), true));
        root.checks
            .insert("b".into(), slot(Some("news"), Some("root.0"), false));
        root.checks
            .insert("c".into(), slot(Some("other"), Some("root.1"), true));
        root.checks
            .insert("d".into(), slot(None, Some("root.0"), true));
        root.checks
            .insert("e".into(), slot(Some("loose"), None, true));
        let mut values = Map::new();
        values.insert("name".into(), Value::String("Ada".into()));
        root.add_checked("root.0", &mut values);
        // Browsers send "on" for checked boxes only; the rest is unchanged.
        assert_eq!(
            Value::Object(values),
            serde_json::json!({"name": "Ada", "agree": "on"})
        );
    }

    fn keystroke(keys: &str, action: Option<&str>, context: &[&str]) -> gpui::KeystrokeEvent {
        gpui::KeystrokeEvent {
            keystroke: gpui::Keystroke::parse(keys).unwrap(),
            input_preference: gpui::InputPreference::KeyBindings,
            action: action.map(|name| {
                Box::new(crate::keymap::RuntimeAction {
                    name: name.to_string().into(),
                }) as Box<dyn gpui::Action>
            }),
            context_stack: context
                .iter()
                .map(|c| gpui::KeyContext::parse(c).unwrap())
                .collect(),
        }
    }

    fn shortcut(event: &str, key: Option<&str>, values: &[(&str, &str)]) -> WindowKey {
        WindowKey {
            event: event.into(),
            key: key.map(Into::into),
            values: attrs(values),
        }
    }

    #[test]
    fn window_shortcuts_send_the_key_and_its_modifiers() {
        let h = crate::bridge::testing::harness();
        let mut root = root();
        root.keydowns = vec![
            shortcut("any", None, &[("tool", "x")]),
            shortcut("only_enter", Some("enter"), &[]),
            shortcut("only_k", Some("K"), &[]),
        ];
        root.on_keystroke(&keystroke("shift-k", None, &[]));
        let events = h.events();
        // `phx-key` matches whatever the case, and others are skipped.
        let names: Vec<_> = events
            .iter()
            .map(|e| e["event"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["any", "only_k"]);
        assert_eq!(
            events[0],
            serde_json::json!({
                "type": "keydown",
                "window": 3,
                "event": "any",
                "values": {
                    "tool": "x", "key": "k",
                    "ctrl": false, "shift": true, "alt": false, "meta": false,
                },
            })
        );
    }

    #[test]
    fn keystrokes_something_handled_or_typed_send_nothing() {
        let h = crate::bridge::testing::harness();
        let mut root = root();
        root.keydowns = vec![shortcut("any", None, &[])];
        // Already dispatched to an action.
        root.on_keystroke(&keystroke("ctrl-z", Some("undo"), &[]));
        // A lone modifier has no key name.
        root.on_keystroke(&keystroke("shift", None, &[]));
        // Typing in an input: plain keys and Alt/Shift chords are the text's.
        root.on_keystroke(&keystroke("a", None, &["Input"]));
        root.on_keystroke(&keystroke("alt-a", None, &["Input"]));
        assert!(h.events().is_empty());
        // Ctrl or Meta chords still fire in an input.
        root.on_keystroke(&keystroke("ctrl-s", None, &["Input"]));
        root.on_keystroke(&keystroke("cmd-s", None, &["Workspace", "Input"]));
        let events = h.events();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["values"]["ctrl"], true);
        assert_eq!(events[1]["values"]["meta"], true);
        // Other contexts are not inputs.
        root.on_keystroke(&keystroke("a", None, &["Editor"]));
        assert_eq!(h.events().len(), 1);
    }

    #[test]
    fn notices_show_their_text() {
        let frame = crate::notice("oops".into());
        assert_eq!(
            text_of(std::slice::from_ref(frame.root.as_ref().unwrap())),
            "oops"
        );
    }
}

/// The view in a headless window: drawing, clicks, typing, lists, forms.
#[cfg(test)]
#[path = "view_tests.rs"]
mod window_tests;

/// Builds the events in `fixtures/events.json` from the inputs the view's own
/// code takes (attributes, keystrokes, checkbox slots, list state).
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use crate::bridge::testing::Harness;

    fn attrs_of(a: &Value) -> BTreeMap<String, String> {
        a["attrs"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect()
    }

    fn window(a: &Value) -> u32 {
        a["window"].as_u64().unwrap_or(0) as u32
    }

    fn root(window: u32) -> Root {
        Root::new(window, Frame::default())
    }

    /// The text fields in `a["texts"]`, then the checkboxes in `a["checks"]`
    /// collected as `form_values` does.
    fn form_values(root: &mut Root, a: &Value) -> Map<String, Value> {
        let form = a["form"].as_str().unwrap();
        let mut values = a["texts"].as_object().unwrap().clone();
        for (i, c) in a["checks"].as_array().unwrap().iter().enumerate() {
            root.checks.insert(
                i.to_string(),
                CheckSlot {
                    checked: c["checked"].as_bool().unwrap(),
                    name: c["name"].as_str().map(Into::into),
                    form: Some(form.to_owned()),
                    ..CheckSlot::default()
                },
            );
        }
        root.add_checked(form, &mut values);
        values
    }

    pub(crate) fn event(h: &Harness, name: &str, a: &Value) -> Option<Value> {
        let w = window(a);
        Some(match name {
            "click" => {
                let attrs = attrs_of(a);
                crate::events::input("click", w, &attrs["phx-click"], values_of(&attrs))
            }
            // A checkbox's click: its `phx-value-*` and `checked`.
            "check_click" => {
                let attrs = attrs_of(a);
                let mut values: Map<String, Value> = values_of(&attrs)
                    .into_iter()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect();
                values.insert("checked".into(), a["checked"].clone());
                crate::events::input("click", w, &attrs["phx-click"], values)
            }
            "drag" => {
                let attrs = attrs_of(a);
                crate::events::drag(
                    w,
                    &attrs["phx-drag"],
                    values_of(&attrs),
                    a["dx"].as_f64().unwrap() as f32,
                    a["dy"].as_f64().unwrap() as f32,
                )
            }
            "keydown" => {
                let mut root = root(w);
                root.keydowns = a["bindings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| WindowKey {
                        event: b["event"].as_str().unwrap().into(),
                        key: b["key"].as_str().map(Into::into),
                        values: b["values"]
                            .as_object()
                            .map(|v| {
                                v.iter()
                                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().into()))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .collect();
                let keystroke = gpui::Keystroke::parse(a["keys"].as_str().unwrap()).unwrap();
                root.on_keystroke(&gpui::KeystrokeEvent {
                    keystroke,
                    input_preference: gpui::InputPreference::KeyBindings,
                    action: None,
                    context_stack: Vec::new(),
                });
                let mut events = h.events();
                assert_eq!(events.len(), 1, "{a}");
                events.remove(0)
            }
            // A form's `change`: its fields, `_target`.
            "form_change" => {
                let mut root = root(w);
                let mut values = form_values(&mut root, a);
                values.insert("_target".into(), a["target"].clone());
                crate::events::input("change", w, a["event"].as_str().unwrap(), values)
            }
            "form_submit" => {
                let mut root = root(w);
                let values = form_values(&mut root, a);
                crate::events::input("submit", w, a["event"].as_str().unwrap(), values)
            }
            // A list asking for rows: `have` is the range already here.
            "rows" => {
                let n = |v: &Value| v.as_u64().unwrap() as usize;
                let mut list = ListState::default();
                list.count = n(&a["count"]);
                let have = (n(&a["have"][0]), n(&a["have"][1]));
                if have.0 < have.1 {
                    list.insert(Segment {
                        id: a["id"].as_str().unwrap().into(),
                        start: have.0,
                        rows: (have.0..have.1).map(|_| Node::Text("x".into())).collect(),
                    });
                }
                let want = list
                    .wanted(n(&a["visible"][0])..n(&a["visible"][1]))
                    .unwrap();
                crate::events::rows(
                    w,
                    a["event"].as_str().unwrap(),
                    a["id"].as_str().unwrap(),
                    want.rows.start,
                    want.rows.end,
                    [want.visible.start, want.visible.end],
                )
            }
            _ => return None,
        })
    }
}
