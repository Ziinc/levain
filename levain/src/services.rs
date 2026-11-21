//! Platform services as calls: notifications, the tray icon, context menus
//! and global shortcuts. Each needs its Tauri plugin (or Tauri feature) in
//! the host; without it, its calls are errors saying so.

use serde::Deserialize;
use serde_json::Value;
use tauri::Manager;

use crate::platform::ItemSpec;

fn missing(what: &str) -> String {
    format!("no {what} in this host")
}

#[derive(Deserialize)]
struct NotificationArgs {
    title: String,
    #[serde(default)]
    body: Option<String>,
}

/// `notification.send`: shows `{title, body?}`.
pub fn notify<R: tauri::Runtime>(app: &tauri::AppHandle<R>, args: &Value) -> Result<Value, String> {
    use tauri_plugin_notification::NotificationExt;
    let args: NotificationArgs = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
    if app
        .try_state::<tauri_plugin_notification::Notification<R>>()
        .is_none()
    {
        return Err(missing("notification plugin"));
    }
    let mut builder = app.notification().builder().title(args.title);
    if let Some(body) = args.body {
        builder = builder.body(body);
    }
    builder.show().map_err(|e| e.to_string())?;
    Ok(Value::Null)
}

/// `notification.permission` and `notification.request_permission`:
/// `granted`, `denied`, `prompt` or `promptWithRationale` (Android).
pub fn notification_permission<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    request: bool,
) -> Result<Value, String> {
    use tauri_plugin_notification::NotificationExt;
    if app
        .try_state::<tauri_plugin_notification::Notification<R>>()
        .is_none()
    {
        return Err(missing("notification plugin"));
    }
    let notification = app.notification();
    let state = if request {
        notification.request_permission()
    } else {
        notification.permission_state()
    }
    .map_err(|e| e.to_string())?;
    serde_json::to_value(state).map_err(|e| e.to_string())
}

/// A menu of `items` (as `menu.set` takes them) for the tray or a popup.
#[cfg(desktop)]
fn menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    items: &[ItemSpec],
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{MenuBuilder, MenuItemBuilder};
    let mut menu = MenuBuilder::new(app);
    for item in items {
        menu = match item {
            ItemSpec::Separator { .. } => menu.separator(),
            ItemSpec::Item { id, label, keys } => {
                let mut item = MenuItemBuilder::with_id(id.as_str(), label);
                if let Some(keys) = keys {
                    item = item.accelerator(crate::platform::accelerator(keys));
                }
                menu.item(&item.build(app)?)
            }
        };
    }
    menu.build()
}

// Phones have neither, but read the arguments all the same.
#[cfg_attr(mobile, allow(dead_code))]
#[derive(Deserialize)]
struct PopupArgs {
    window: u32,
    items: Vec<ItemSpec>,
}

/// `menu.popup`: a context menu at the pointer in `window`. Its clicks are
/// `menu` events, as the menu bar's are.
pub fn popup<R: tauri::Runtime>(app: &tauri::AppHandle<R>, args: &Value) -> Result<Value, String> {
    let args: PopupArgs = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
    #[cfg(desktop)]
    {
        let window = app
            .get_window(&crate::windows::label(args.window))
            .ok_or("no such window")?;
        let menu = menu(app, &args.items).map_err(|e| e.to_string())?;
        window.popup_menu(&menu).map_err(|e| e.to_string())?;
    }
    #[cfg(mobile)]
    let _ = (app, args);
    Ok(Value::Null)
}

// Phones have neither, but read the arguments all the same.
#[cfg_attr(mobile, allow(dead_code))]
#[derive(Deserialize)]
struct TrayArgs {
    /// A PNG file; the app's icon when absent.
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    tooltip: Option<String>,
    #[serde(default)]
    items: Vec<ItemSpec>,
}

/// The tray icon's id.
#[cfg(desktop)]
const TRAY: &str = "levain";

/// `tray.set`: shows the app's tray icon, or changes it. Clicking it sends
/// a `tray` event; its menu's items send `menu` events. Desktop only.
pub fn tray<R: tauri::Runtime>(app: &tauri::AppHandle<R>, args: &Value) -> Result<Value, String> {
    let args: TrayArgs = serde_json::from_value(args.clone()).map_err(|e| e.to_string())?;
    #[cfg(desktop)]
    {
        let icon = match &args.icon {
            Some(path) => Some(tauri::image::Image::from_path(path).map_err(|e| e.to_string())?),
            None => app.default_window_icon().cloned(),
        };
        let menu = (!args.items.is_empty())
            .then(|| menu(app, &args.items))
            .transpose()
            .map_err(|e| e.to_string())?;
        let result = match app.tray_by_id(TRAY) {
            Some(tray) => tray
                .set_icon(icon)
                .and_then(|()| tray.set_tooltip(args.tooltip.as_deref()))
                .and_then(|()| tray.set_menu(menu)),
            None => {
                let mut builder = tauri::tray::TrayIconBuilder::with_id(TRAY)
                    .on_tray_icon_event(|_, event| on_tray_event(&event));
                if let Some(icon) = icon {
                    builder = builder.icon(icon);
                }
                if let Some(tooltip) = &args.tooltip {
                    builder = builder.tooltip(tooltip);
                }
                if let Some(menu) = &menu {
                    builder = builder.menu(menu).show_menu_on_left_click(false);
                }
                builder.build(app).map(drop)
            }
        };
        result.map_err(|e| e.to_string())?;
    }
    #[cfg(mobile)]
    let _ = (app, args);
    Ok(Value::Null)
}

/// `tray.remove`.
pub fn tray_remove<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Result<Value, String> {
    #[cfg(desktop)]
    app.remove_tray_by_id(TRAY);
    #[cfg(mobile)]
    let _ = app;
    Ok(Value::Null)
}

#[cfg(desktop)]
fn on_tray_event(event: &tauri::tray::TrayIconEvent) {
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
    let name = match event {
        TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } => "click",
        TrayIconEvent::DoubleClick {
            button: MouseButton::Left,
            ..
        } => "double-click",
        _ => return,
    };
    crate::bridge::emit(&crate::events::tray(name));
}

/// The keys (as the app wrote them) of each registered global shortcut, by
/// its id.
#[cfg(desktop)]
static SHORTCUTS: std::sync::Mutex<Option<std::collections::HashMap<u32, String>>> =
    std::sync::Mutex::new(None);

/// The global shortcut plugin, sending each press as a `shortcut` event.
#[cfg(desktop)]
pub fn shortcut_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_global_shortcut::ShortcutState;
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|_, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            let keys = SHORTCUTS
                .lock()
                .unwrap()
                .as_ref()
                .and_then(|s| s.get(&shortcut.id()).cloned());
            if let Some(keys) = keys {
                crate::bridge::emit(&crate::events::shortcut(&keys));
            }
        })
        .build()
}

/// `shortcut.register` / `shortcut.unregister` `{keys}`, in GPUI's syntax
/// (`secondary-shift-k`). Desktop only.
pub fn shortcut<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    args: &Value,
    register: bool,
) -> Result<Value, String> {
    let keys = args["keys"].as_str().ok_or("no `keys`")?;
    #[cfg(desktop)]
    {
        use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
        if app
            .try_state::<tauri_plugin_global_shortcut::GlobalShortcut<R>>()
            .is_none()
        {
            return Err(missing("global shortcut plugin"));
        }
        let shortcut: Shortcut = crate::platform::accelerator(keys)
            .parse()
            .map_err(|e| format!("`{keys}`: {e}"))?;
        let shortcuts = app.global_shortcut();
        let mut known = SHORTCUTS.lock().unwrap();
        let known = known.get_or_insert_default();
        if register {
            if !known.contains_key(&shortcut.id()) {
                shortcuts.register(shortcut).map_err(|e| e.to_string())?;
            }
            known.insert(shortcut.id(), keys.to_owned());
        } else if known.remove(&shortcut.id()).is_some() {
            shortcuts.unregister(shortcut).map_err(|e| e.to_string())?;
        }
    }
    #[cfg(mobile)]
    let _ = (app, register, keys);
    Ok(Value::Null)
}

/// What a call to these takes, without running it: the error for arguments
/// it cannot take.
#[cfg(test)]
pub(crate) fn args_error(call: &str, args: &Value) -> Option<String> {
    let read = |result: Result<(), serde_json::Error>| result.err().map(|e| e.to_string());
    match call {
        "notification.send" => {
            read(serde_json::from_value::<NotificationArgs>(args.clone()).map(drop))
        }
        "menu.popup" => read(serde_json::from_value::<PopupArgs>(args.clone()).map(drop)),
        "tray.set" => read(serde_json::from_value::<TrayArgs>(args.clone()).map(drop)),
        "shortcut.register" | "shortcut.unregister" => args["keys"]
            .as_str()
            .map_or(Some("no `keys`".into()), |_| None),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arguments_are_checked_before_anything_runs() {
        assert_eq!(
            args_error("notification.send", &json!({"title": "Done"})),
            None
        );
        assert_eq!(
            args_error("notification.send", &json!({"body": "x"})),
            Some("missing field `title`".into())
        );
        assert_eq!(
            args_error(
                "menu.popup",
                &json!({"window": 0, "items": [{"id": "copy", "label": "Copy"}]})
            ),
            None
        );
        assert_eq!(args_error("tray.set", &json!({})), None);
        assert_eq!(
            args_error("shortcut.register", &json!({})),
            Some("no `keys`".into())
        );
    }

    #[test]
    fn hosts_without_the_plugins_say_so() {
        let app = tauri::test::mock_app();
        assert_eq!(
            notify(app.handle(), &json!({"title": "Hi"})),
            Err("no notification plugin in this host".into())
        );
        assert_eq!(
            notification_permission(app.handle(), false),
            Err("no notification plugin in this host".into())
        );
        assert_eq!(
            shortcut(app.handle(), &json!({"keys": "secondary-shift-k"}), true),
            Err("no global shortcut plugin in this host".into())
        );
    }

    #[test]
    fn a_context_menu_needs_its_window() {
        let app = tauri::test::mock_app();
        assert_eq!(
            popup(app.handle(), &json!({"window": 3, "items": []})),
            Err("no such window".into())
        );
    }

    // Building a tray needs the desktop's indicator library (on Linux,
    // libayatana-appindicator3), so only what fails before that runs here.
    #[test]
    fn a_tray_icon_that_cannot_be_read_is_an_error_and_removing_none_is_not() {
        let app = tauri::test::mock_app();
        assert!(tray(app.handle(), &json!({"icon": "/no/such/icon.png"})).is_err());
        assert_eq!(tray_remove(app.handle()), Ok(Value::Null));
    }
}
