//! What happens to the app's windows and the app itself, as events for the
//! app: a window resized, moved, focused, closed or dropped on, a phone app
//! sent to the background and back, and URLs (deep links, files) opened with
//! the app.

use std::collections::HashSet;
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::bridge;

/// Windows whose close the app decides on (`window.prevent_close`).
static PREVENTED: Mutex<Option<HashSet<u32>>> = Mutex::new(None);

/// Makes closing window `id` ask the app (`close-requested`) instead of
/// closing it, or stops that.
pub fn prevent_close(id: u32, prevent: bool) {
    let mut prevented = PREVENTED.lock().unwrap();
    let prevented = prevented.get_or_insert_default();
    if prevent {
        prevented.insert(id);
    } else {
        prevented.remove(&id);
    }
}

pub(crate) fn is_prevented(id: u32) -> bool {
    PREVENTED
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|p| p.contains(&id))
}

/// The number of the window labelled `label` (`windows::label`'s inverse).
pub fn window_id(label: &str) -> Option<u32> {
    match label {
        "main" => Some(0),
        _ => label.strip_prefix("window-")?.parse().ok(),
    }
}

/// The app's event for what happened to window `id`, at `scale` (physical
/// pixels per logical one), if it has one. A close request is handled by
/// [`on_window_event`], which can stop it.
pub fn window_event(id: u32, scale: f64, event: &tauri::WindowEvent) -> Option<Value> {
    use tauri::{DragDropEvent, WindowEvent};
    let logical = |n: f64| n / scale;
    Some(match event {
        WindowEvent::Resized(size) => crate::events::window(
            id,
            "resized",
            json!({"width": logical(size.width.into()), "height": logical(size.height.into())}),
        ),
        WindowEvent::Moved(at) => crate::events::window(
            id,
            "moved",
            json!({"x": logical(at.x.into()), "y": logical(at.y.into())}),
        ),
        WindowEvent::Focused(true) => crate::events::window(id, "focused", json!({})),
        WindowEvent::Focused(false) => crate::events::window(id, "blurred", json!({})),
        WindowEvent::Destroyed => crate::events::window(id, "closed", json!({})),
        WindowEvent::DragDrop(DragDropEvent::Drop { paths, position }) => crate::events::window(
            id,
            "drop",
            json!({
                "paths": paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
                "x": logical(position.x),
                "y": logical(position.y),
            }),
        ),
        #[cfg(mobile)]
        WindowEvent::Suspended => crate::events::window(id, "suspended", json!({})),
        #[cfg(mobile)]
        WindowEvent::Resumed => crate::events::window(id, "resumed", json!({})),
        _ => return None,
    })
}

/// Tells the app what happened to `window`. A close the app asked to decide
/// on (`window.prevent_close`) is stopped and sent as `close-requested`.
pub fn on_window_event<R: tauri::Runtime>(window: &tauri::Window<R>, event: &tauri::WindowEvent) {
    let Some(id) = window_id(window.label()) else {
        return;
    };
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        if is_prevented(id) {
            api.prevent_close();
            bridge::emit(&crate::events::window(id, "close-requested", json!({})));
        }
        return;
    }
    if matches!(event, tauri::WindowEvent::Destroyed) {
        prevent_close(id, false);
    }
    let scale = window.scale_factor().unwrap_or(1.);
    if let Some(event) = window_event(id, scale, event) {
        bridge::emit(&event);
    }
}

/// Tells the app about URLs it was opened with: deep links on phones and
/// macOS, files opened with it on macOS.
pub fn on_run_event<R: tauri::Runtime>(_app: &tauri::AppHandle<R>, event: &tauri::RunEvent) {
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "android"))]
    if let tauri::RunEvent::Opened { urls } = event {
        bridge::emit(&crate::events::opened(urls.iter().map(|u| u.as_str())));
    }
    #[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
    let _ = event;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::{DragDropEvent, PhysicalPosition, PhysicalSize, WindowEvent};

    #[test]
    fn window_ids_are_read_from_labels() {
        for id in [0, 1, 42] {
            assert_eq!(window_id(&crate::windows::label(id)), Some(id));
        }
        assert_eq!(window_id("window-x"), None);
        assert_eq!(window_id("other"), None);
    }

    #[test]
    fn window_events_are_in_logical_pixels() {
        assert_eq!(
            window_event(1, 2., &WindowEvent::Resized(PhysicalSize::new(800, 600))),
            Some(
                json!({"type": "window", "window": 1, "event": "resized", "width": 400.0, "height": 300.0})
            )
        );
        assert_eq!(
            window_event(0, 2., &WindowEvent::Moved(PhysicalPosition::new(10, -4))),
            Some(json!({"type": "window", "window": 0, "event": "moved", "x": 5.0, "y": -2.0}))
        );
        assert_eq!(
            window_event(
                0,
                1.,
                &WindowEvent::DragDrop(DragDropEvent::Drop {
                    paths: vec!["/tmp/a.txt".into(), "/tmp/b c.png".into()],
                    position: PhysicalPosition::new(12., 30.),
                })
            ),
            Some(json!({"type": "window", "window": 0, "event": "drop",
                        "paths": ["/tmp/a.txt", "/tmp/b c.png"], "x": 12.0, "y": 30.0}))
        );
    }

    #[test]
    fn focus_and_closing_are_named() {
        let named = |event| window_event(3, 1., &event).map(|e| e["event"].clone());
        assert_eq!(named(WindowEvent::Focused(true)), Some(json!("focused")));
        assert_eq!(named(WindowEvent::Focused(false)), Some(json!("blurred")));
        assert_eq!(named(WindowEvent::Destroyed), Some(json!("closed")));
        assert_eq!(
            named(WindowEvent::DragDrop(DragDropEvent::Leave)),
            None,
            "only drops are sent"
        );
    }

    #[test]
    fn closing_a_window_forgets_that_the_app_decides_on_it() {
        prevent_close(7, true);
        assert!(is_prevented(7));
        prevent_close(7, false);
        assert!(!is_prevented(7));
    }

    #[test]
    fn a_closed_window_is_sent_and_no_longer_prevented() {
        let h = crate::bridge::testing::harness();
        let app = tauri::test::mock_app();
        let window = crate::windows::build(app.handle(), 5, "Five", (100., 100.)).unwrap();
        prevent_close(5, true);
        on_window_event(&window, &WindowEvent::Destroyed);
        assert_eq!(
            h.events(),
            [json!({"type": "window", "window": 5, "event": "closed"})]
        );
        assert!(!is_prevented(5));
    }
}
