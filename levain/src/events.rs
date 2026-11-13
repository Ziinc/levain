//! Every event the host sends a runtime (JSON, through its `EventSink`).
//! `fixtures/events.json` holds one of each, for every language to decode.

use serde::Serialize;
use serde_json::{Value, json};

/// A user's input on an element: `click`, `change`, `submit`, `keydown` or
/// `pick`, with the event named in its `phx-*` attribute.
pub fn input(kind: &str, window: u32, event: &str, values: impl Serialize) -> Value {
    json!({"type": kind, "window": window, "event": event, "values": values})
}

/// A press dragged past the click threshold.
pub fn drag(window: u32, event: &str, values: impl Serialize, dx: f32, dy: f32) -> Value {
    json!({"type": "drag", "window": window, "event": event, "values": values, "dx": dx, "dy": dy})
}

/// A virtualized list wants rows `start..end`; `visible` is on screen.
pub fn rows(
    window: u32,
    event: &str,
    id: &str,
    start: usize,
    end: usize,
    visible: [usize; 2],
) -> Value {
    json!({"type": "rows", "window": window, "event": event, "id": id, "start": start, "end": end, "visible": visible})
}

/// A keymap action fired.
pub fn action(window: u32, action: &str) -> Value {
    json!({"type": "action", "window": window, "action": action})
}

/// The app's appearance (`light` or `dark`).
pub fn appearance(window: u32, appearance: &str) -> Value {
    json!({"type": "appearance", "window": window, "appearance": appearance})
}

/// The host lost track of a window's tree: send it whole.
pub fn resync(window: u32) -> Value {
    json!({"type": "resync", "window": window})
}

/// A menu item was clicked; menus belong to window 0.
pub fn menu(event: &str) -> Value {
    json!({"type": "menu", "window": 0, "event": event})
}

/// A platform call's result.
pub fn reply(reference: Value, result: Value) -> Value {
    json!({"type": "reply", "ref": reference, "result": result})
}

/// A platform call that failed.
pub fn reply_error(reference: Value, error: String) -> Value {
    json!({"type": "reply", "ref": reference, "result": null, "error": error})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build(name: &str, a: &Value) -> Value {
        let s = |k: &str| a[k].as_str().unwrap();
        let w = || a["window"].as_u64().unwrap() as u32;
        let values = || a["values"].as_object().unwrap().clone();
        let n = |k: &str| a[k].as_u64().unwrap() as usize;
        match name {
            "input" => input(s("type"), w(), s("event"), values()),
            "drag" => drag(
                w(),
                s("event"),
                values(),
                a["dx"].as_f64().unwrap() as f32,
                a["dy"].as_f64().unwrap() as f32,
            ),
            "rows" => rows(
                w(),
                s("event"),
                s("id"),
                n("start"),
                n("end"),
                [
                    a["visible"][0].as_u64().unwrap() as usize,
                    a["visible"][1].as_u64().unwrap() as usize,
                ],
            ),
            "action" => action(w(), s("action")),
            "appearance" => appearance(w(), s("appearance")),
            "resync" => resync(w()),
            "menu" => menu(s("event")),
            "reply" => reply(a["ref"].clone(), a["result"].clone()),
            "reply_error" => reply_error(a["ref"].clone(), s("error").into()),
            other => panic!("no event builder {other}"),
        }
    }

    #[test]
    fn conformance_fixture() {
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../../fixtures/events.json")).unwrap();
        let mut types = std::collections::BTreeSet::new();
        for case in &cases {
            let built = build(case["build"].as_str().unwrap(), &case["args"]);
            assert_eq!(built, case["json"], "{}", case["name"]);
            types.insert(built["type"].as_str().unwrap().to_owned());
        }
        // One case for every type the host sends.
        let all = [
            "action",
            "appearance",
            "change",
            "click",
            "drag",
            "keydown",
            "menu",
            "pick",
            "reply",
            "resync",
            "rows",
            "submit",
        ];
        assert_eq!(types.into_iter().collect::<Vec<_>>(), all);
    }
}
