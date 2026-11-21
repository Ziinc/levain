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

/// Something happened to a window: `resized` (`width`, `height`), `moved`
/// (`x`, `y`), `focused`, `blurred`, `close-requested`, `closed`, `drop`
/// (`paths`, `x`, `y`), or on phones `suspended` and `resumed`. `extra` is an
/// object of the event's own keys.
pub fn window(window: u32, event: &str, extra: Value) -> Value {
    let mut json = json!({"type": "window", "window": window, "event": event});
    if let (Some(json), Value::Object(extra)) = (json.as_object_mut(), extra) {
        json.extend(extra);
    }
    json
}

/// The app was opened with URLs: deep links, or files on macOS.
pub fn opened<'a>(urls: impl IntoIterator<Item = &'a str>) -> Value {
    json!({"type": "open", "urls": urls.into_iter().collect::<Vec<_>>()})
}

/// The tray icon was clicked (`click`) or double-clicked (`double-click`).
pub fn tray(event: &str) -> Value {
    json!({"type": "tray", "event": event})
}

/// A global shortcut (`shortcut.register`) was pressed; `keys` as registered.
pub fn shortcut(keys: &str) -> Value {
    json!({"type": "shortcut", "keys": keys})
}

/// The app was started again while running (`single_instance`): the new
/// process's arguments and working directory. It exits; this one stays.
pub fn instance(args: &[String], cwd: &str) -> Value {
    json!({"type": "instance", "args": args, "cwd": cwd})
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

    fn build(h: &crate::bridge::testing::Harness, name: &str, a: &Value) -> Value {
        if let Some(built) = crate::view::fixture::event(h, name, a)
            .or_else(|| crate::pick::fixture::event(h, name, a))
            .or_else(|| crate::platform::fixture::event(name, a))
        {
            return built;
        }
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
            "window" => window(w(), s("event"), a["extra"].clone()),
            "tray" => tray(s("event")),
            "shortcut" => shortcut(s("keys")),
            "instance" => instance(
                &a["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|x| x.as_str().unwrap().to_owned())
                    .collect::<Vec<_>>(),
                s("cwd"),
            ),
            "opened" => opened(
                a["urls"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|u| u.as_str().unwrap()),
            ),
            "reply" => reply(a["ref"].clone(), a["result"].clone()),
            "reply_error" => reply_error(a["ref"].clone(), s("error").into()),
            other => panic!("no event builder {other}"),
        }
    }

    #[test]
    fn conformance_fixture() {
        let h = crate::bridge::testing::harness();
        let cases: Vec<Value> =
            serde_json::from_str(include_str!("../../fixtures/events.json")).unwrap();
        let mut types = std::collections::BTreeSet::new();
        let mut names = std::collections::BTreeSet::new();
        for case in &cases {
            assert!(
                names.insert(case["name"].as_str().unwrap()),
                "{}",
                case["name"]
            );
            let built = build(&h, case["build"].as_str().unwrap(), &case["args"]);
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
            "instance",
            "keydown",
            "menu",
            "open",
            "pick",
            "reply",
            "resync",
            "rows",
            "shortcut",
            "submit",
            "tray",
            "window",
        ];
        assert_eq!(types.into_iter().collect::<Vec<_>>(), all);
    }
}
