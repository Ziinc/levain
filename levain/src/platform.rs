//! Platform calls from Elixir (`Baguette.Clipboard`, `Baguette.Window`,
//! `Baguette.Menu`, `Baguette.App`), run on the UI thread. GPUI does the
//! clipboard and appearance; Tauri, which is the platform under GPUI here,
//! does windows, menus and opening URLs.

use serde::Deserialize;
use serde_json::{Value, json};
use tauri::Manager;
use tauri_plugin_gpui::gpui::{self, ClipboardItem, WindowAppearance};

use crate::bridge;

/// `{"call": name, "ref": n, "args": {...}}`; the reply carries `ref`.
#[derive(Deserialize, Debug)]
pub struct Request {
    pub call: String,
    #[serde(rename = "ref")]
    pub reference: u64,
    #[serde(default)]
    pub args: Value,
}

#[derive(Deserialize, Debug, Default)]
struct WindowArgs {
    window: u32,
    title: Option<String>,
    x: Option<f64>,
    y: Option<f64>,
    width: Option<f64>,
    height: Option<f64>,
}

#[derive(Deserialize, Debug, PartialEq)]
pub struct MenuSpec {
    pub title: String,
    pub items: Vec<ItemSpec>,
}

#[derive(Deserialize, Debug, PartialEq)]
#[serde(untagged)]
pub enum ItemSpec {
    Separator {
        separator: bool,
    },
    Item {
        id: String,
        label: String,
        #[serde(default)]
        keys: Option<String>,
    },
}

/// Runs `json` and sends Elixir the reply: `result`, or `error` and a nil
/// result.
pub fn handle(json: &[u8], app: &tauri::AppHandle, cx: &mut gpui::App) {
    let request = match parse(json) {
        Ok(request) => request,
        Err((error, reference)) => {
            crate::log_line(&format!("baguette: invalid platform call: {error}"));
            // Answer it if it has a ref, so its caller doesn't wait.
            if let Some(reference) = reference {
                bridge::emit(&crate::events::reply_error(
                    reference.into(),
                    format!("invalid platform call: {error}"),
                ));
            }
            return;
        }
    };
    bridge::emit(&reply_for(&request, run(&request, app, cx)));
}

/// The request in `json`; or why it isn't one, and its `ref` if it has one.
fn parse(json: &[u8]) -> Result<Request, (serde_json::Error, Option<u64>)> {
    serde_json::from_slice(json).map_err(|error| {
        let reference = serde_json::from_slice::<Value>(json)
            .ok()
            .and_then(|call| call["ref"].as_u64());
        (error, reference)
    })
}

/// The reply event for how `request` went.
fn reply_for(request: &Request, outcome: Result<Value, String>) -> Value {
    match outcome {
        Ok(result) => crate::events::reply(request.reference.into(), result),
        Err(error) => crate::events::reply_error(
            request.reference.into(),
            format!("{}: {error}", request.call),
        ),
    }
}

fn run(request: &Request, app: &tauri::AppHandle, cx: &mut gpui::App) -> Result<Value, String> {
    let args = || -> Result<WindowArgs, String> {
        serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())
    };
    let text = |key: &str| {
        request.args[key]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("no `{key}`"))
    };
    match request.call.as_str() {
        "clipboard.read" => Ok(json!(cx.read_from_clipboard().and_then(|item| item.text()))),
        "clipboard.write" => {
            cx.write_to_clipboard(ClipboardItem::new_string(text("text")?));
            Ok(Value::Null)
        }
        "app.appearance" => Ok(json!(appearance(crate::portal::appearance(cx)))),
        "app.open_url" => {
            use tauri_plugin_opener::OpenerExt;
            app.opener()
                .open_url(text("url")?, None::<&str>)
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "menu.set" => {
            let menus: Vec<MenuSpec> =
                serde_json::from_value(request.args["menus"].clone()).map_err(|e| e.to_string())?;
            #[cfg(desktop)]
            set_menus(app, &menus).map_err(|e| e.to_string())?;
            // Phones have no menu bar.
            #[cfg(mobile)]
            let _ = menus;
            Ok(Value::Null)
        }
        "window.open" => {
            let args = args()?;
            let title = args.title.unwrap_or_default();
            let size = (args.width.unwrap_or(800.), args.height.unwrap_or(600.));
            crate::windows::open(app, args.window, &title, size, Default::default())
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "window.close" => {
            if let Some(window) = window(app, args()?.window) {
                window.close().map_err(|e| e.to_string())?;
            }
            Ok(Value::Null)
        }
        "window.set_title" => {
            let args = args()?;
            let window = window(app, args.window).ok_or("no such window")?;
            window
                .set_title(&args.title.unwrap_or_default())
                .map_err(|e| e.to_string())?;
            Ok(Value::Null)
        }
        "window.set_bounds" => {
            let args = args()?;
            let window = window(app, args.window).ok_or("no such window")?;
            let info = info(&window).map_err(|e| e.to_string())?;
            let at = |key: &str, given: Option<f64>| given.or(info[key].as_f64()).unwrap_or(0.);
            if args.width.is_some() || args.height.is_some() {
                let size =
                    tauri::LogicalSize::new(at("width", args.width), at("height", args.height));
                window.set_size(size).map_err(|e| e.to_string())?;
            }
            if args.x.is_some() || args.y.is_some() {
                let position = tauri::LogicalPosition::new(at("x", args.x), at("y", args.y));
                window.set_position(position).map_err(|e| e.to_string())?;
            }
            Ok(Value::Null)
        }
        "window.info" => match window(app, args()?.window) {
            Some(window) => info(&window).map_err(|e| e.to_string()),
            None => Ok(Value::Null),
        },
        other => Err(format!("no platform call `{other}`")),
    }
}

pub fn appearance(appearance: WindowAppearance) -> &'static str {
    match appearance {
        WindowAppearance::Dark | WindowAppearance::VibrantDark => "dark",
        WindowAppearance::Light | WindowAppearance::VibrantLight => "light",
    }
}

fn window(app: &tauri::AppHandle, id: u32) -> Option<tauri::Window> {
    app.get_window(&crate::windows::label(id))
}

/// Its title and bounds, in logical pixels.
fn info(window: &tauri::Window) -> tauri::Result<Value> {
    let scale = window.scale_factor()?;
    let size = window.inner_size()?.to_logical::<f64>(scale);
    let position = window.outer_position()?.to_logical::<f64>(scale);
    Ok(json!({
        "title": window.title()?,
        "x": position.x,
        "y": position.y,
        "width": size.width,
        "height": size.height,
    }))
}

#[cfg(desktop)]
fn set_menus(app: &tauri::AppHandle, menus: &[MenuSpec]) -> tauri::Result<()> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
    let mut bar = MenuBuilder::new(app);
    for menu in menus {
        let mut submenu = SubmenuBuilder::new(app, &menu.title);
        for item in &menu.items {
            submenu = match item {
                ItemSpec::Separator { .. } => submenu.separator(),
                ItemSpec::Item { id, label, keys } => {
                    let mut item = MenuItemBuilder::with_id(id.as_str(), label);
                    if let Some(keys) = keys {
                        item = item.accelerator(accelerator(keys));
                    }
                    submenu.item(&item.build(app)?)
                }
            };
        }
        bar = bar.item(&submenu.build()?);
    }
    app.set_menu(bar.build()?)?;
    Ok(())
}

/// Sends each menu click to window 0's view, as an event named by the
/// item's id.
#[cfg(desktop)]
pub fn on_menu_event(event: tauri::menu::MenuEvent) {
    bridge::emit(&crate::events::menu(&event.id().0));
}

/// A keystroke in GPUI's syntax (`secondary-shift-p`) as a Tauri
/// accelerator (`CmdOrCtrl+Shift+P`).
#[cfg_attr(mobile, allow(dead_code))]
pub fn accelerator(keys: &str) -> String {
    let parts: Vec<&str> = match keys.strip_suffix("--") {
        // `ctrl--` is Ctrl and the minus key.
        Some(rest) => rest.split('-').chain(["-"]).collect(),
        None => keys.split('-').collect(),
    };
    parts
        .into_iter()
        .map(|part| match part {
            "secondary" => "CmdOrCtrl".to_owned(),
            "cmd" | "platform" => "Cmd".to_owned(),
            "ctrl" => "Ctrl".to_owned(),
            "alt" => "Alt".to_owned(),
            "shift" => "Shift".to_owned(),
            "fn" => "Fn".to_owned(),
            key => {
                let mut chars = key.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().chain(chars).collect(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_requests() {
        let request: Request = serde_json::from_str(
            r#"{"call": "window.set_title", "ref": 7, "args": {"window": 2, "title": "Jobs"}}"#,
        )
        .unwrap();
        assert_eq!(
            (request.call.as_str(), request.reference),
            ("window.set_title", 7)
        );
        let args: WindowArgs = serde_json::from_value(request.args).unwrap();
        assert_eq!((args.window, args.title.as_deref()), (2, Some("Jobs")));
    }

    #[test]
    fn reads_menus() {
        let menus: Vec<MenuSpec> = serde_json::from_str(
            r#"[{"title": "File", "items": [
                {"id": "new", "label": "New", "keys": "secondary-n"},
                {"separator": true},
                {"id": "quit", "label": "Quit"}]}]"#,
        )
        .unwrap();
        assert_eq!(menus[0].items.len(), 3);
        assert_eq!(menus[0].items[1], ItemSpec::Separator { separator: true });
        assert!(matches!(
            &menus[0].items[2],
            ItemSpec::Item { keys: None, .. }
        ));
    }

    #[test]
    fn writes_keystrokes_as_accelerators() {
        assert_eq!(accelerator("secondary-n"), "CmdOrCtrl+N");
        assert_eq!(accelerator("ctrl-shift-p"), "Ctrl+Shift+P");
        assert_eq!(accelerator("cmd-,"), "Cmd+,");
        assert_eq!(accelerator("alt-f4"), "Alt+F4");
        assert_eq!(accelerator("ctrl--"), "Ctrl+-");
    }

    #[test]
    fn names_appearances() {
        assert_eq!(appearance(WindowAppearance::VibrantDark), "dark");
        assert_eq!(appearance(WindowAppearance::Light), "light");
    }

    #[test]
    fn names_every_appearance() {
        assert_eq!(appearance(WindowAppearance::Dark), "dark");
        assert_eq!(appearance(WindowAppearance::VibrantLight), "light");
    }

    #[test]
    fn requests_default_to_no_args() {
        let request = parse(br#"{"call": "clipboard.read", "ref": 1}"#).unwrap();
        assert_eq!(request.args, Value::Null);
        // `ref` is required, and a string is not one.
        assert!(parse(br#"{"call": "clipboard.read"}"#).is_err());
        assert!(parse(br#"{"call": "x", "ref": "1"}"#).is_err());
    }

    #[test]
    fn invalid_calls_keep_their_ref_to_be_answered() {
        // Valid JSON that isn't a request: no `call`.
        let (error, reference) = parse(br#"{"ref": 12, "args": {}}"#).unwrap_err();
        assert_eq!(reference, Some(12));
        assert!(error.to_string().contains("call"), "{error}");
        // Not JSON at all: nothing to answer.
        assert_eq!(parse(b"not json").unwrap_err().1, None);
        // A ref that isn't a number can't be answered either.
        assert_eq!(parse(br#"{"ref": "x"}"#).unwrap_err().1, None);
    }

    #[test]
    fn replies_carry_the_ref_and_the_result_or_error() {
        let request =
            parse(br#"{"call": "window.info", "ref": 5, "args": {"window": 1}}"#).unwrap();
        assert_eq!(
            reply_for(&request, Ok(json!({"title": "T"}))),
            json!({"type": "reply", "ref": 5, "result": {"title": "T"}})
        );
        assert_eq!(
            reply_for(&request, Err("no such window".into())),
            json!({
                "type": "reply",
                "ref": 5,
                "result": null,
                "error": "window.info: no such window",
            })
        );
    }

    #[test]
    fn menu_items_need_an_id_and_a_label() {
        assert!(
            serde_json::from_str::<MenuSpec>(r#"{"title": "File", "items": [{"id": "new"}]}"#)
                .is_err()
        );
        let menu: MenuSpec = serde_json::from_str(
            r#"{"title": "File", "items": [{"id": "new", "label": "New", "keys": null}]}"#,
        )
        .unwrap();
        assert_eq!(
            menu.items,
            [ItemSpec::Item {
                id: "new".into(),
                label: "New".into(),
                keys: None
            }]
        );
    }

    #[test]
    fn window_args_need_only_a_window() {
        let args: WindowArgs =
            serde_json::from_value(json!({"window": 1, "width": 640.5})).unwrap();
        assert_eq!(
            (args.window, args.width, args.height),
            (1, Some(640.5), None)
        );
        assert!(serde_json::from_value::<WindowArgs>(json!({"title": "x"})).is_err());
    }

    #[test]
    fn accelerators_keep_unknown_parts_and_capitalise_keys() {
        assert_eq!(accelerator("platform-q"), "Cmd+Q");
        assert_eq!(accelerator("fn-f1"), "Fn+F1");
        assert_eq!(accelerator("escape"), "Escape");
        assert_eq!(accelerator("secondary-shift--"), "CmdOrCtrl+Shift+-");
    }
}

/// Builds `reply` events for `fixtures/events.json` through the request
/// parser and [`reply_for`].
#[cfg(test)]
pub(crate) mod fixture {
    // Also the tests of `fixtures/calls.json`, the requests.
    use super::*;

    pub(crate) fn event(name: &str, a: &Value) -> Option<Value> {
        Some(match name {
            // A request that ran: `ok` is its result, `err` its error.
            "reply_for" => {
                let request = Request {
                    call: a["call"].as_str().unwrap().into(),
                    reference: a["ref"].as_u64().unwrap(),
                    args: Value::Null,
                };
                let outcome = match a.get("err") {
                    Some(err) => Err(err.as_str().unwrap().to_owned()),
                    None => Ok(a["ok"].clone()),
                };
                reply_for(&request, outcome)
            }
            // Text that isn't a request, but has a `ref` to answer, as
            // `handle` answers it.
            "reply_invalid" => {
                let (error, reference) = parse(a["request"].as_str().unwrap().as_bytes())
                    .expect_err("an invalid request");
                crate::events::reply_error(
                    reference.expect("a ref").into(),
                    format!("invalid platform call: {error}"),
                )
            }
            _ => return None,
        })
    }

    /// What `run` does with `request`'s arguments before it needs a window
    /// or the platform: the error it fails with, if any.
    fn args_error(request: &Request) -> Option<String> {
        let text = |key: &str| {
            request.args[key]
                .as_str()
                .map_or(Some(format!("no `{key}`")), |_| None)
        };
        let window = || {
            serde_json::from_value::<WindowArgs>(request.args.clone())
                .err()
                .map(|e| e.to_string())
        };
        match request.call.as_str() {
            "clipboard.read" | "app.appearance" => None,
            "clipboard.write" => text("text"),
            "app.open_url" => text("url"),
            "menu.set" => serde_json::from_value::<Vec<MenuSpec>>(request.args["menus"].clone())
                .err()
                .map(|e| e.to_string()),
            "window.open" | "window.close" | "window.set_title" | "window.set_bounds"
            | "window.info" => window(),
            other => Some(format!("no platform call `{other}`")),
        }
    }

    const CALLS: [&str; 10] = [
        "clipboard.read",
        "clipboard.write",
        "app.appearance",
        "app.open_url",
        "menu.set",
        "window.open",
        "window.close",
        "window.set_title",
        "window.set_bounds",
        "window.info",
    ];

    fn calls_fixture() -> Vec<Value> {
        serde_json::from_str(include_str!("../../fixtures/calls.json")).unwrap()
    }

    #[test]
    fn every_call_fixture_request_parses() {
        let mut seen = std::collections::BTreeSet::new();
        for case in calls_fixture().iter().filter(|c| c.get("reply").is_none()) {
            let json = serde_json::to_vec(&case["json"]).unwrap();
            let request = parse(&json).unwrap_or_else(|(e, _)| panic!("{}: {e}", case["name"]));
            assert_eq!(request.call, case["json"]["call"], "{}", case["name"]);
            assert_eq!(request.reference, case["json"]["ref"], "{}", case["name"]);
            assert_eq!(request.args, case["json"]["args"], "{}", case["name"]);
            // Its arguments are ones the call accepts.
            assert_eq!(args_error(&request), None, "{}", case["name"]);
            seen.insert(request.call);
        }
        assert_eq!(seen.into_iter().collect::<Vec<_>>(), {
            let mut all = CALLS.map(String::from).to_vec();
            all.sort();
            all
        });
    }

    #[test]
    fn the_fixture_names_the_calls_run_handles() {
        // `run`'s match arms, read from the source.
        let source = include_str!("platform.rs")
            .split("#[cfg(test)]")
            .next()
            .unwrap();
        let handled: std::collections::BTreeSet<&str> = source
            .lines()
            .filter_map(|l| l.trim().strip_prefix('"')?.split_once("\" =>"))
            .map(|(name, _)| name)
            .filter(|n| n.contains('.'))
            .collect();
        assert_eq!(handled, CALLS.into_iter().collect());
    }

    #[test]
    fn invalid_call_fixture_requests_get_the_stated_reply() {
        let mut answered = 0;
        for case in calls_fixture().iter().filter(|c| c.get("reply").is_some()) {
            let name = &case["name"];
            let reply = match case["raw"].as_str() {
                // Text that isn't a request: `handle`'s answer, if it has a ref.
                Some(raw) => match parse(raw.as_bytes()) {
                    Ok(_) => panic!("{name}: parses"),
                    Err((error, reference)) => reference.map_or(Value::Null, |reference| {
                        crate::events::reply_error(
                            reference.into(),
                            format!("invalid platform call: {error}"),
                        )
                    }),
                },
                // A request `run` fails on, before it needs a window.
                None => {
                    let json = serde_json::to_vec(&case["json"]).unwrap();
                    let request = parse(&json).unwrap_or_else(|(e, _)| panic!("{name}: {e}"));
                    let error = args_error(&request).unwrap_or_else(|| panic!("{name}: no error"));
                    reply_for(&request, Err(error))
                }
            };
            assert_eq!(reply, case["reply"], "{name}");
            answered += usize::from(!reply.is_null());
        }
        assert!(answered > 0);
    }
}
