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
    let request: Request = match serde_json::from_slice(json) {
        Ok(request) => request,
        Err(error) => {
            crate::log_line(&format!("baguette: invalid platform call: {error}"));
            // Answer it if it has a ref, so its caller doesn't wait.
            let reference = serde_json::from_slice::<Value>(json)
                .ok()
                .and_then(|call| call["ref"].as_u64());
            if let Some(reference) = reference {
                bridge::emit(&crate::events::reply_error(
                    reference.into(),
                    format!("invalid platform call: {error}"),
                ));
            }
            return;
        }
    };
    let reply = match run(&request, app, cx) {
        Ok(result) => crate::events::reply(request.reference.into(), result),
        Err(error) => crate::events::reply_error(
            request.reference.into(),
            format!("{}: {error}", request.call),
        ),
    };
    bridge::emit(&reply);
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
}
