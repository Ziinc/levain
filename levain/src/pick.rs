//! Native open and save dialogs for `phx-pick-file` and `phx-pick-save`.
//!
//! Test hook: with `BAGUETTE_PICK_FILE` set, every picker answers with that
//! path at once instead of opening a dialog, as end-to-end tests cannot drive
//! the platform's dialogs.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use crate::bridge;

static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Keeps the app handle the dialogs open from. Call once, in setup.
pub fn install(app: tauri::AppHandle) {
    let _ = APP.set(app);
}

/// A picker element's binding: what to send, and how to filter.
pub struct Pick {
    pub window: u32,
    pub event: String,
    pub values: BTreeMap<String, String>,
    pub save: bool,
    /// File extensions without dots, from `accept="csv,xlsx"`.
    pub accept: Vec<String>,
    pub default_name: Option<String>,
}

impl Pick {
    pub fn from_attrs(window: u32, attrs: &BTreeMap<String, String>) -> Option<Self> {
        let (event, save) = match (attrs.get("phx-pick-file"), attrs.get("phx-pick-save")) {
            (Some(event), _) => (event.clone(), false),
            (None, Some(event)) => (event.clone(), true),
            (None, None) => return None,
        };
        Some(Self {
            window,
            event,
            values: crate::view::values_of(attrs),
            save,
            accept: attrs
                .get("accept")
                .map(|a| {
                    a.split(',')
                        .map(|e| e.trim().trim_start_matches('.').to_owned())
                        .filter(|e| !e.is_empty())
                        .collect()
                })
                .unwrap_or_default(),
            default_name: attrs.get("default-name").cloned(),
        })
    }

    /// Opens the dialog; a chosen path is sent to the view, a cancel is not.
    pub fn open(self) {
        if let Some(path) = std::env::var_os("BAGUETTE_PICK_FILE") {
            return self.send(path.to_string_lossy().into_owned());
        }
        #[cfg(mobile)]
        crate::log_line("baguette: file pickers are not available on mobile yet");
        #[cfg(desktop)]
        {
            use tauri_plugin_dialog::DialogExt;
            let Some(app) = APP.get() else {
                return crate::log_line("baguette: no app handle for the file dialog");
            };
            let mut dialog = app.dialog().file();
            if !self.accept.is_empty() {
                let extensions: Vec<&str> = self.accept.iter().map(String::as_str).collect();
                dialog = dialog.add_filter(self.accept.join(", "), &extensions);
            }
            if let Some(name) = &self.default_name {
                dialog = dialog.set_file_name(name);
            }
            if self.save {
                dialog.save_file(move |path| {
                    if let Some(path) = path {
                        self.send(path.to_string());
                    }
                });
            } else {
                dialog.pick_file(move |path| {
                    if let Some(path) = path {
                        self.send(path.to_string());
                    }
                });
            }
        }
    }

    fn send(&self, path: String) {
        let mut values: serde_json::Map<String, serde_json::Value> = self
            .values
            .iter()
            .map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone())))
            .collect();
        values.insert("path".into(), serde_json::Value::String(path));
        bridge::emit(&crate::events::input(
            "pick",
            self.window,
            &self.event,
            values,
        ));
    }
}
