use gpui_kit::{Focusable, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px};
use serde_json::json;

use super::tests::ops;
use super::*;
use crate::bridge::testing::harness;
use crate::testing::{draw, open, show};

fn column(children: &str) -> String {
    format!(r#"{{"root": {{"t": "div", "a": {{"class": "flex flex-col"}}, "c": [{children}]}}}}"#)
}

fn click(cx: &mut VisualTestContext, x: f32, y: f32) {
    cx.simulate_click(point(px(x), px(y)), Modifiers::none());
}

/// Gives the input at `path` focus.
fn focus_input(view: &Entity<Root>, path: &str, cx: &mut VisualTestContext) {
    let handle = view.read_with(cx, |root, cx| {
        root.inputs[path].state.read(cx).focus_handle(cx)
    });
    cx.update(|window, cx| window.focus(&handle, cx));
}

/// Types `text` into whatever has focus, one keystroke at a time.
fn type_text(cx: &mut VisualTestContext, text: &str) {
    use gpui_kit::test::TestWindowExt;
    cx.update(|window, cx| window.input(text, cx));
}

#[gpui_kit::test]
fn opening_a_window_tells_the_app_its_appearance(cx: &mut TestAppContext) {
    let h = harness();
    let (view, cx) = open(cx, r#"{"root": "hello"}"#);
    assert_eq!(
        h.events(),
        [json!({"type": "appearance", "window": 0, "appearance": "light"})]
    );
    assert_eq!(
        view.read_with(cx, |root, _| root.frame.root.clone()),
        Some(Node::Text("hello".into()))
    );
}

#[gpui_kit::test]
fn clicking_sends_the_event_and_its_values(cx: &mut TestAppContext) {
    let h = harness();
    let (_view, cx) = open(
        cx,
        &column(
            r#"{"t": "button", "a": {"class": "w-20 h-10", "phx-click": "go", "phx-value-id": "7"}, "c": ["Go"]},
               {"t": "button", "a": {"class": "w-20 h-10", "phx-click": "never", "disabled": ""}, "c": ["No"]}"#,
        ),
    );
    h.events();
    click(cx, 40., 20.);
    assert_eq!(
        h.events(),
        [json!({"type": "click", "window": 0, "event": "go", "values": {"id": "7"}})]
    );
    // A disabled element does not click.
    click(cx, 40., 60.);
    assert!(h.events().is_empty());
}

#[gpui_kit::test]
fn dragging_sends_how_far_the_pointer_went(cx: &mut TestAppContext) {
    let h = harness();
    let (_view, cx) = open(
        cx,
        &column(
            r#"{"t": "div", "a": {"class": "w-20 h-10", "phx-drag": "move", "phx-value-id": "a"}}"#,
        ),
    );
    h.events();
    let none = Modifiers::none();
    // Released over the element.
    cx.simulate_mouse_down(point(px(10.), px(10.)), MouseButton::Left, none);
    cx.simulate_mouse_up(point(px(40.), px(25.)), MouseButton::Left, none);
    assert_eq!(
        h.events(),
        [json!({"type": "drag", "window": 0, "event": "move",
                "values": {"id": "a"}, "dx": 30.0, "dy": 15.0})]
    );
    // Released anywhere else.
    cx.simulate_mouse_down(point(px(10.), px(10.)), MouseButton::Left, none);
    cx.simulate_mouse_up(point(px(300.), px(200.)), MouseButton::Left, none);
    let events = h.events();
    assert_eq!(
        (events[0]["dx"].as_f64(), events[0]["dy"].as_f64()),
        (Some(290.), Some(190.))
    );
    // A press that barely moved is a click.
    cx.simulate_mouse_down(point(px(10.), px(10.)), MouseButton::Left, none);
    cx.simulate_mouse_up(point(px(11.), px(11.)), MouseButton::Left, none);
    assert!(h.events().is_empty());
    // A release with no press does nothing.
    cx.simulate_mouse_up(point(px(11.), px(11.)), MouseButton::Left, none);
    assert!(h.events().is_empty());
}

const FORM: &str = r#"{"t": "form", "a": {"class": "flex flex-col", "phx-submit": "save", "phx-change": "edit"}, "c": [
    {"t": "div", "a": {"class": "w-40 h-10"}, "c": [{"t": "input", "a": {"name": "who", "value": "Ada"}}]},
    {"t": "div", "a": {"class": "w-40 h-10"}, "c": [{"t": "input", "a": {"type": "checkbox", "name": "agree", "checked": ""}}]},
    {"t": "button", "a": {"type": "submit", "class": "w-20 h-10"}, "c": ["Save"]}
]}"#;

#[gpui_kit::test]
fn a_submit_button_sends_the_forms_named_values(cx: &mut TestAppContext) {
    let h = harness();
    let (view, cx) = open(cx, &column(FORM));
    h.events();
    view.read_with(cx, |root, _| {
        assert_eq!(root.forms["root.0"].submit.as_deref(), Some("save"));
        assert_eq!(root.forms["root.0"].change.as_deref(), Some("edit"));
        assert_eq!(root.inputs.len(), 1);
        assert_eq!(root.checks.len(), 1);
    });
    click(cx, 40., 100.);
    assert_eq!(
        h.events(),
        [json!({"type": "submit", "window": 0, "event": "save",
                "values": {"who": "Ada", "agree": "on"}})]
    );
}

#[gpui_kit::test]
fn typing_in_a_form_sends_its_change_and_enter_submits_it(cx: &mut TestAppContext) {
    let h = harness();
    let (view, cx) = open(cx, &column(FORM));
    h.events();
    focus_input(&view, "root.0.0.0", cx);
    type_text(cx, "!");

    assert_eq!(
        h.events(),
        [json!({"type": "change", "window": 0, "event": "edit",
                "values": {"who": "Ada!", "agree": "on", "_target": "who"}})]
    );
    cx.simulate_keystrokes("enter");
    let events = h.events();
    assert_eq!(events[0]["type"], "submit");
    assert_eq!(events[0]["event"], "save");
    assert_eq!(events[0]["values"], json!({"who": "Ada!", "agree": "on"}));
}

#[gpui_kit::test]
fn ticking_a_checkbox_in_a_form_sends_the_forms_change(cx: &mut TestAppContext) {
    let h = harness();
    let (view, cx) = open(cx, &column(FORM));
    h.events();
    click(cx, 8., 48.);
    assert_eq!(
        h.events(),
        [json!({"type": "change", "window": 0, "event": "edit",
                "values": {"who": "Ada", "_target": "agree"}})]
    );
    view.read_with(cx, |root, _| assert!(!root.checks["root.0.1.0"].checked));
    draw(cx);
    click(cx, 8., 48.);
    let events = h.events();
    assert_eq!(events[0]["values"]["agree"], "on", "{events:?}");
}

#[gpui_kit::test]
fn a_checkbox_sends_its_click_and_the_apps_checked_wins(cx: &mut TestAppContext) {
    let h = harness();
    let checkbox = |checked: &str| {
        column(&format!(
            r#"{{"t": "div", "a": {{"class": "w-40 h-10"}}, "c": [{{"t": "input", "a": {{"type": "checkbox",
                "phx-click": "toggle", "phx-value-id": "3", "aria-label": "Done"{checked}}}}}]}}"#
        ))
    };
    let (view, cx) = open(cx, &checkbox(""));
    h.events();
    click(cx, 8., 8.);
    assert_eq!(
        h.events(),
        [json!({"type": "click", "window": 0, "event": "toggle",
                "values": {"id": "3", "checked": true}})]
    );
    let checked =
        |cx: &mut VisualTestContext| view.read_with(cx, |r, _| r.checks["root.0.0"].checked);
    assert!(checked(cx));
    // The app renders it unchecked again: its word stands.
    show(&view, cx, &checkbox(r#", "checked": """#));
    assert!(checked(cx));
    show(&view, cx, &checkbox(""));
    assert!(!checked(cx));
}

#[gpui_kit::test]
fn inputs_outside_a_form_send_their_own_change(cx: &mut TestAppContext) {
    let h = harness();
    let input = |value: &str, extra: &str| {
        column(&format!(
            r#"{{"t": "div", "a": {{"class": "w-40 h-10"}}, "c": [{{"t": "input", "a": {{"value": "{value}",
                "phx-change": "typed", "placeholder": "Type"{extra}}}}}]}}"#
        ))
    };
    let (view, cx) = open(cx, &input("a", ""));
    h.events();
    focus_input(&view, "root.0.0", cx);
    type_text(cx, "b");
    // No `name`: the value is called `value`.
    assert_eq!(
        h.events(),
        [json!({"type": "change", "window": 0, "event": "typed",
                "values": {"value": "ab", "_target": "value"}})]
    );
    // Enter outside a form submits nothing.
    cx.simulate_keystrokes("enter");
    assert!(h.events().iter().all(|e| e["type"] != "submit"));

    let text = |cx: &mut VisualTestContext| {
        view.read_with(cx, |root, cx| {
            root.inputs["root.0.0"].state.read(cx).value().to_string()
        })
    };
    // Typed text stands when the app renders the value it rendered before...
    show(&view, cx, &input("a", ""));
    assert_eq!(text(cx), "ab");
    // ...and gives way to a new one, which is not sent back.
    show(
        &view,
        cx,
        &input("zed", r#", "name": "q", "type": "password""#),
    );
    assert_eq!(text(cx), "zed");
    assert!(h.events().is_empty());
    // The placeholder follows the app too.
    show(
        &view,
        cx,
        &input("zed", r#", "placeholder": "New""#).replace("\"Type\"", "\"New\""),
    );
    view.read_with(cx, |root, _| {
        assert_eq!(root.inputs["root.0.0"].placeholder, "New")
    });
}

#[gpui_kit::test]
fn a_picker_sends_the_chosen_path(cx: &mut TestAppContext) {
    let h = harness();
    let (_view, cx) = open(
        cx,
        &column(
            r#"{"t": "button", "a": {"class": "w-20 h-10", "phx-pick-file": "chosen", "phx-value-kind": "csv"}, "c": ["Open"]}"#,
        ),
    );
    h.events();
    let hook = crate::pick::hook::set("/tmp/in.csv");
    click(cx, 40., 20.);
    drop(hook);
    assert_eq!(
        h.events(),
        [json!({"type": "pick", "window": 0, "event": "chosen",
                "values": {"kind": "csv", "path": "/tmp/in.csv"}})]
    );
}

#[gpui_kit::test]
fn every_kind_of_element_draws_and_registers_what_it_needs(cx: &mut TestAppContext) {
    let h = harness();
    let (view, cx) = open(
        cx,
        &column(
            r##"
            {"t": "div", "a": {"class": "w-20 h-10 overflow-scroll overflow-x-auto overflow-y-scroll bg-primary",
                "tabindex": "1", "key-context": "Pane", "title": "A tip", "role": "group",
                "phx-window-keydown": "esc", "phx-key": "Escape", "phx-value-from": "pane",
                "animate": "pulse", "duration": "60000"}, "c": ["pane"]},
            {"t": "text", "a": {"class": "text-sm"}, "c": ["label"]},
            {"t": "h2", "c": ["Heading"]},
            {"t": "a", "a": {"aria-label": "Home"}, "c": ["link"]},
            {"t": "div", "a": {"aria-checked": "mixed", "disabled": "", "class": "opacity-50 disabled:opacity-20"}},
            {"t": "div", "a": {"aria-checked": "true"}},
            {"t": "div", "a": {"aria-checked": "false"}},
            {"t": "icon", "a": {"name": "save", "class": "size-4 text-primary"}},
            {"t": "icon", "a": {"name": "no-such-icon"}},
            {"t": "icon", "a": {"name": "loader", "animate": "spin", "duration": "500"}},
            {"t": "line", "a": {"x1": "0", "y1": "0", "x2": "30", "y2": "20", "class": "stroke-primary"}},
            {"t": "line", "a": {"x2": "30", "y2": "20", "curve": "", "stroke-width": "4"}},
            {"t": "canvas", "a": {"class": "w-20 h-10"}, "c": [
                {"t": "rect", "a": {"width": "5", "height": "5", "class": "fill-primary"}},
                {"t": "circle", "a": {"cx": "9", "cy": "9", "r": "3"}},
                {"t": "path", "a": {"d": "M 0 0 L 10 10 Z", "class": "fill-primary stroke-primary"}},
                "text is not a shape"]},
            {"t": "kit-tag", "a": {"variant": "success", "pill": "", "outline": "", "small": ""}, "c": ["ok"]},
            {"t": "kit-progress", "a": {"value": "40", "aria-label": "Done"}},
            {"t": "kit-spinner", "a": {"size": "small"}},
            {"t": "kit-skeleton", "a": {"class": "w-10 h-2", "secondary": ""}},
            {"t": "kit-kbd", "a": {"keys": "cmd-k", "outline": ""}},
            {"t": "kit-kbd", "a": {"keys": ""}}
            "##,
        ),
    );
    // The first frame, with its icon draws, tells the app nothing.
    assert!(h.events().iter().all(|e| e["type"] == "appearance"));
    view.read_with(cx, |root, _| {
        assert_eq!(root.keydowns.len(), 1);
        assert_eq!(root.keydowns[0].event, "esc");
        assert_eq!(root.keydowns[0].key.as_deref(), Some("Escape"));
        assert_eq!(root.keydowns[0].values["from"], "pane");
        assert!(root.focus.contains_key("root.0"));
        assert!(root.animations.len() >= 1);
    });
}

#[gpui_kit::test]
fn window_shortcuts_hear_keys_nothing_else_took(cx: &mut TestAppContext) {
    let h = harness();
    let (_view, cx) = open(
        cx,
        r#"{"root": {"t": "div", "a": {"phx-window-keydown": "shortcut", "phx-key": "x"}},
            "keymap": [{"keys": "ctrl-k", "action": "palette"}, {"keys": "", "action": "bad"}]}"#,
    );
    h.events();
    cx.simulate_keystrokes("x");
    let events = h.events();
    assert_eq!(events.len(), 1, "{events:?}");
    assert_eq!(events[0]["event"], "shortcut");
    assert_eq!(events[0]["values"]["key"], "x");
    // Another key is not this binding's.
    cx.simulate_keystrokes("y");
    assert!(h.events().is_empty());
    // A keymap action goes to the app as one, and is not also a keydown.
    cx.simulate_keystrokes("ctrl-k");
    assert_eq!(
        h.events(),
        [json!({"type": "action", "window": 0, "action": "palette"})]
    );
}

#[gpui_kit::test]
fn a_new_keymap_replaces_the_old_and_an_unchanged_one_is_left_alone(cx: &mut TestAppContext) {
    let h = harness();
    let frame = |keys: &str| {
        format!(
            r#"{{"root": {{"t": "div", "a": {{"key-context": "Pane"}}}},
                "keymap": [{{"keys": "{keys}", "action": "go"}}]}}"#
        )
    };
    let (view, cx) = open(cx, &frame("ctrl-j"));
    h.events();
    cx.simulate_keystrokes("ctrl-j");
    assert_eq!(h.events().len(), 1);
    show(&view, cx, &frame("ctrl-j"));
    cx.simulate_keystrokes("ctrl-j");
    assert_eq!(h.events().len(), 1);
    show(&view, cx, &frame("ctrl-l"));
    cx.simulate_keystrokes("ctrl-j");
    assert!(h.events().is_empty());
    cx.simulate_keystrokes("ctrl-l");
    assert_eq!(h.events()[0]["action"], "go");
    // A binding GPUI can't parse is dropped, and the rest stay.
    show(
        &view,
        cx,
        r#"{"root": null, "keymap": [{"keys": "ctrl-m", "action": "m", "context": "A &&"},
                       {"keys": "ctrl-n", "action": "n"}]}"#,
    );
    cx.simulate_keystrokes("ctrl-m ctrl-n");
    let events = h.events();
    assert_eq!(
        events
            .iter()
            .map(|e| e["action"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["n"]
    );
}

#[gpui_kit::test]
fn a_binding_with_a_context_fires_only_inside_it(cx: &mut TestAppContext) {
    let h = harness();
    let (view, cx) = open(
        cx,
        r#"{"root": {"t": "div", "a": {"key-context": "Pane", "tabindex": "0"}},
            "keymap": [{"keys": "ctrl-p", "action": "in_pane", "context": "Pane"}]}"#,
    );
    h.events();
    // Focus is on the window's body, outside the pane.
    cx.simulate_keystrokes("ctrl-p");
    assert!(h.events().is_empty());
    let pane = view.read_with(cx, |root, _| root.focus["root"].clone());
    cx.update(|window, cx| window.focus(&pane, cx));
    draw(cx);
    cx.simulate_keystrokes("ctrl-p");
    assert_eq!(
        h.events(),
        [json!({"type": "action", "window": 0, "action": "in_pane"})]
    );
}

#[gpui_kit::test]
fn the_apps_theme_colours_the_native_components(cx: &mut TestAppContext) {
    use gpui_kit::component::Theme as KitTheme;
    let _h = harness();
    let (view, cx) = open(cx, r#"{"root": "x"}"#);
    let primary = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| KitTheme::global(cx).light_theme.colors.primary.clone())
    };
    let before = primary(cx);
    show(
        &view,
        cx,
        r##"{"theme": {"primary": "#123456", "primary-hover": "#234567", "input": "#345678",
            "background": "#fafafa", "foreground": "#111111", "danger": "#ff0000"}, "root": "x"}"##,
    );
    assert_eq!(primary(cx).as_deref(), Some("#123456"));
    assert_ne!(before.as_deref(), Some("#123456"));
    let (hover, input, danger) = cx.update(|_, cx| {
        let colors = &KitTheme::global(cx).dark_theme.colors;
        (
            colors.primary_hover.clone(),
            colors.input.clone(),
            colors.danger.clone(),
        )
    });
    assert_eq!(
        (hover.as_deref(), input.as_deref(), danger.as_deref()),
        (Some("#234567"), Some("#345678"), Some("#ff0000"))
    );
}

fn list_frame(rows: &str) -> String {
    format!(
        r#"{{"root": {{"t": "div", "a": {{"class": "flex flex-col"}}, "c": [
            {{"t": "list", "a": {{"id": "jobs", "rows": "5", "phx-rows": "want", "row-height": "20", "class": "w-40 h-24"}}}}]}},
           "lists": {{"jobs": {{"start": 0, "rows": [{rows}]}}}}}}"#
    )
}

fn row(id: usize) -> String {
    format!(
        r#"{{"t": "button", "a": {{"class": "w-20 h-5", "phx-click": "open", "phx-value-id": "{id}"}}, "c": ["row {id}"]}}"#
    )
}

#[gpui_kit::test]
fn a_list_draws_the_rows_it_has_and_asks_for_the_rest(cx: &mut TestAppContext) {
    let h = harness();
    let have = format!("{}, {}, {}", row(0), row(1), row(2));
    let (view, cx) = open(cx, &list_frame(&have));
    // The list is new, so a first frame's rows are asked for again...
    assert!(
        h.events()
            .iter()
            .any(|e| e["type"] == "rows" && e["start"] == 0)
    );
    // ...and a frame after it brings them.
    show(&view, cx, &list_frame(&have));
    assert_eq!(
        h.events()
            .into_iter()
            .filter(|e| e["type"] == "rows")
            .collect::<Vec<_>>(),
        [
            json!({"type": "rows", "window": 0, "event": "want", "id": "jobs",
                "start": 3, "end": 5, "visible": [0, 5]})
        ]
    );
    view.read_with(cx, |root, _| {
        let list = &root.lists["jobs"];
        assert_eq!(list.count, 5);
        assert_eq!(list.event.as_deref(), Some("want"));
        assert_eq!(list.path, "root.0");
        assert_eq!(list.rows.len(), 3);
    });
    // A row is an element of its own.
    click(cx, 40., 50.);
    assert_eq!(
        h.events(),
        [json!({"type": "click", "window": 0, "event": "open", "values": {"id": "2"}})]
    );

    // The app answers; a drawn list asks for nothing more.
    let rest = format!("{}, {}", row(3), row(4));
    let segment = match serde_json::from_str(&format!(
        r#"{{"rows": {{"id": "jobs", "start": 3, "rows": [{rest}]}}}}"#
    )) {
        Ok(Message::Rows { rows }) => rows,
        _ => panic!("not rows"),
    };
    view.update(cx, |root, cx| root.rows(segment, cx));
    draw(cx);
    assert!(h.events().is_empty());
    view.read_with(cx, |root, _| assert_eq!(root.lists["jobs"].rows.len(), 5));
    click(cx, 40., 90.);
    assert_eq!(h.events()[0]["values"], json!({"id": "4"}));

    // Rows for a list that isn't there are dropped.
    let stray = Segment {
        id: "other".into(),
        start: 0,
        rows: vec![Node::Text("x".into())],
    };
    view.update(cx, |root, cx| root.rows(stray, cx));
    view.read_with(cx, |root, _| assert!(!root.lists.contains_key("other")));
}

#[gpui_kit::test]
fn a_list_not_rendered_any_more_drops_its_state(cx: &mut TestAppContext) {
    let _h = harness();
    let (view, cx) = open(cx, &list_frame(&row(0)));
    view.read_with(cx, |root, _| assert!(root.lists.contains_key("jobs")));
    show(&view, cx, r#"{"root": "gone"}"#);
    view.read_with(cx, |root, _| {
        assert!(root.lists.is_empty() && root.inputs.is_empty() && root.checks.is_empty());
    });
}

#[gpui_kit::test]
fn a_list_row_holds_inputs_and_checkboxes_that_outlive_a_render(cx: &mut TestAppContext) {
    let h = harness();
    let rows = r#"{"t": "div", "a": {"class": "flex flex-row w-40 h-5"}, "c": [
        {"t": "input", "a": {"type": "checkbox", "name": "done", "phx-click": "tick"}},
        {"t": "input", "a": {"name": "title", "value": "t"}},
        {"t": "div", "a": {"animate": "fade-in", "duration": "60000", "tabindex": "0"}}]}"#;
    let (view, cx) = open(cx, &list_frame(rows));
    show(&view, cx, &list_frame(rows));
    h.events();
    view.read_with(cx, |root, _| {
        assert_eq!(root.checks.len(), 1);
        assert_eq!(root.inputs.len(), 1);
        assert!(root.inputs.contains_key("root.0.rows.0.1"));
        assert!(root.focus.contains_key("root.0.rows.0.2"));
    });
    // Rendering the frame around the list again keeps the row's state.
    draw(cx);
    view.read_with(cx, |root, _| {
        assert_eq!(root.checks.len(), 1);
        assert_eq!(root.inputs.len(), 1);
    });
}

#[gpui_kit::test]
fn patches_change_what_is_shown_and_resync_when_they_do_not_fit(cx: &mut TestAppContext) {
    let h = harness();
    let (view, cx) = open(cx, &column(r#""a", "b""#));
    h.events();
    view.update(cx, |root, cx| {
        root.patch(
            ops(r#"[{"op": "replace", "at": [1], "node": "B"}]"#),
            HashMap::new(),
            cx,
        )
    });
    draw(cx);
    view.read_with(cx, |root, _| {
        let Some(Node::Element { children, .. }) = &root.frame.root else {
            panic!("no root");
        };
        assert_eq!(children[1], Node::Text("B".into()));
    });
    assert!(h.events().is_empty());
    view.update(cx, |root, cx| {
        root.patch(
            ops(r#"[{"op": "replace", "at": [9], "node": "?"}]"#),
            HashMap::new(),
            cx,
        )
    });
    assert_eq!(h.events(), [json!({"type": "resync", "window": 0})]);
    // Until a frame arrives, patches are dropped, not answered.
    view.update(cx, |root, cx| root.patch(Vec::new(), HashMap::new(), cx));
    assert!(h.events().is_empty());
    show(&view, cx, &column(r#""c""#));
    view.read_with(cx, |root, _| assert!(!root.desynced));
}
