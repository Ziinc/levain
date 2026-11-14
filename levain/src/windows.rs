//! The app's windows: each a Tauri window with a GPUI `Root` in it,
//! numbered as the app numbers them (window 0 is the main one).

use std::collections::HashMap;

use tauri_plugin_gpui::GpuiWindowExt;
use tauri_plugin_gpui::gpui::{self, AppContext, Global, WeakEntity};

use crate::bridge::RenderMsg;
use crate::view::{Frame, Message, Root};

/// Each open window's root, and what the app sent windows not open yet.
#[derive(Default)]
struct Windows {
    roots: HashMap<u32, WeakEntity<Root>>,
    queued: HashMap<u32, Vec<Vec<u8>>>,
}

impl Global for Windows {}

/// The Tauri label of window `id`.
pub fn label(id: u32) -> String {
    match id {
        0 => "main".into(),
        id => format!("window-{id}"),
    }
}

/// Opens window `id`, showing `frame` until the app renders it.
pub fn open(
    app: &tauri::AppHandle,
    id: u32,
    title: &str,
    (width, height): (f64, f64),
    frame: Frame,
) -> Result<(), Box<dyn std::error::Error>> {
    let window = tauri::WindowBuilder::new(app, label(id)).title(title);
    // A phone's window is its screen; iOS would take the size literally.
    #[cfg(desktop)]
    let window = window.inner_size(width, height);
    #[cfg(mobile)]
    let _ = (width, height);
    let window = window.build()?;
    // gpui-kit's Root hosts its components (focus, theme, popovers) and,
    // for Playwright, observes what each frame paints.
    window.attach_gpui_view(Default::default(), move |window, cx| {
        let root = cx.new(|_| Root::new(id, frame));
        // Window shortcuts (`phx-window-keydown`) hear every keystroke,
        // wherever focus is, once GPUI has dispatched it.
        let keys = root.downgrade();
        cx.observe_keystrokes(move |event, _, cx| {
            if let Some(root) = keys.upgrade() {
                root.read(cx).on_keystroke(event);
            }
        })
        .detach();
        // GTK's theme changed; the portal's colour scheme, once known, wins.
        let appearance = window.observe_window_appearance(move |_, cx| emit_appearance(id, cx));
        root.update(cx, |root, _| root.keep(appearance));
        let queued = {
            let windows = cx.default_global::<Windows>();
            windows.roots.insert(id, root.downgrade());
            windows.queued.remove(&id).unwrap_or_default()
        };
        for json in queued {
            deliver(&root, &json, cx);
        }
        cx.new(|cx| gpui_kit::base::Root::new(root, window, cx))
    })?;
    Ok(())
}

/// Tells window `id`'s view the app's appearance.
fn emit_appearance(id: u32, cx: &gpui::App) {
    crate::bridge::emit(&crate::events::appearance(
        id,
        crate::platform::appearance(crate::portal::appearance(cx)),
    ));
}

/// Tells every open window's view that the app's appearance changed.
pub fn appearance_changed(cx: &mut gpui::App) {
    let ids: Vec<u32> = cx
        .default_global::<Windows>()
        .roots
        .keys()
        .copied()
        .collect();
    for id in ids {
        emit_appearance(id, cx);
    }
}

/// Shows a render on its window, or keeps it until that window opens.
pub fn route(msg: RenderMsg, cx: &mut gpui::App) {
    let windows = cx.default_global::<Windows>();
    match windows
        .roots
        .get(&msg.window)
        .and_then(|root| root.upgrade())
    {
        Some(root) => deliver(&root, &msg.json, cx),
        None => {
            windows.roots.remove(&msg.window);
            if !queue(windows.queued.entry(msg.window).or_default(), msg.json) {
                crate::bridge::emit(&crate::events::resync(msg.window));
            }
        }
    }
}

/// The most a closed window keeps: a frame and the patches and rows after it.
const MAX_QUEUED: usize = 64;

/// Keeps `json` for a window that isn't open. Only the latest frame matters,
/// with what follows it; a patch or rows without a frame to follow, or one
/// more than the queue holds, drops the queue and returns false: The app is
/// then asked for its whole frame instead.
fn queue(queue: &mut Vec<Vec<u8>>, json: Vec<u8>) -> bool {
    if is_frame(&json) {
        queue.clear();
    } else if queue.is_empty() || queue.len() >= MAX_QUEUED {
        queue.clear();
        return false;
    }
    queue.push(json);
    true
}

fn is_frame(json: &[u8]) -> bool {
    matches!(serde_json::from_slice(json), Ok(Message::Frame(_)))
}

fn deliver(root: &gpui::Entity<Root>, json: &[u8], cx: &mut gpui::App) {
    match serde_json::from_slice::<Message>(json) {
        Ok(Message::Frame(frame)) => root.update(cx, |root, cx| root.show(frame, cx)),
        Ok(Message::Rows { rows }) => root.update(cx, |root, cx| root.rows(rows, cx)),
        Ok(Message::Patch { patch, lists }) => {
            root.update(cx, |root, cx| root.patch(patch, lists, cx))
        }
        Err(error) => {
            let frame = crate::notice(format!("Invalid render: {error}"));
            root.update(cx, |root, cx| root.show(frame, cx))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_windows() {
        assert_eq!(label(0), "main");
        assert_eq!(label(3), "window-3");
    }

    #[test]
    fn queues_a_frame_and_what_follows_it() {
        let mut q = Vec::new();
        assert!(!queue(&mut q, br#"{"patch": []}"#.to_vec()));
        assert!(q.is_empty());
        assert!(queue(&mut q, br#"{"root": null}"#.to_vec()));
        for _ in 1..MAX_QUEUED {
            assert!(queue(&mut q, br#"{"patch": []}"#.to_vec()));
        }
        assert_eq!(q.len(), MAX_QUEUED);
        assert!(!queue(&mut q, br#"{"patch": []}"#.to_vec()));
        assert!(q.is_empty());
    }

    #[test]
    fn tells_frames_from_patches_and_rows() {
        assert!(is_frame(br#"{"theme": {}, "root": null}"#));
        assert!(!is_frame(br#"{"patch": []}"#));
        assert!(!is_frame(
            br#"{"rows": {"id": "a", "start": 0, "rows": []}}"#
        ));
    }

    #[test]
    fn a_frame_replaces_everything_queued() {
        let mut q = Vec::new();
        assert!(queue(&mut q, br#"{"root": "first"}"#.to_vec()));
        assert!(queue(&mut q, br#"{"patch": []}"#.to_vec()));
        assert!(queue(
            &mut q,
            br#"{"rows": {"id": "a", "start": 0, "rows": []}}"#.to_vec()
        ));
        assert!(queue(&mut q, br#"{"root": "second"}"#.to_vec()));
        assert_eq!(q, [br#"{"root": "second"}"#.to_vec()]);
    }

    #[test]
    fn a_frame_restarts_a_full_queue() {
        let mut q = Vec::new();
        assert!(queue(&mut q, br#"{"root": null}"#.to_vec()));
        for _ in 1..MAX_QUEUED {
            assert!(queue(&mut q, br#"{"patch": []}"#.to_vec()));
        }
        // A frame needs no queue to follow, and makes room.
        assert!(queue(&mut q, br#"{"root": null}"#.to_vec()));
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn rows_without_a_frame_ask_for_one() {
        let mut q = Vec::new();
        let rows = br#"{"rows": {"id": "a", "start": 0, "rows": []}}"#;
        assert!(!queue(&mut q, rows.to_vec()));
        assert!(q.is_empty());
    }

    #[test]
    fn what_is_not_json_is_not_a_frame() {
        assert!(!is_frame(b"nope"));
    }
}
