//! Native open and save dialogs for `phx-pick-file` and `phx-pick-save`.
//!
//! Test hook: with `LEVAIN_PICK_FILE` set, every picker answers with that
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

/// The path every picker answers with instead of opening a dialog, if the
/// test hook is set.
fn forced() -> Option<std::ffi::OsString> {
    #[cfg(test)]
    if let Some(path) = hook::get() {
        return Some(path.into());
    }
    std::env::var_os("LEVAIN_PICK_FILE")
}

/// `LEVAIN_PICK_FILE` for tests, without changing the process's environment
/// (which other threads read).
#[cfg(test)]
pub(crate) mod hook {
    static PATH: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

    pub(crate) fn get() -> Option<String> {
        PATH.lock().unwrap().clone()
    }

    /// Answers every picker with `path` until the guard is dropped.
    pub(crate) fn set(path: &str) -> Guard {
        *PATH.lock().unwrap() = Some(path.to_owned());
        Guard
    }

    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            *PATH.lock().unwrap() = None;
        }
    }
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
        if let Some(path) = forced() {
            return self.send(path.to_string_lossy().into_owned());
        }
        #[cfg(mobile)]
        crate::log_line("levain: file pickers are not available on mobile yet");
        #[cfg(desktop)]
        {
            use tauri_plugin_dialog::DialogExt;
            let Some(app) = APP.get() else {
                return crate::log_line("levain: no app handle for the file dialog");
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::testing::harness;
    use serde_json::json;

    fn attrs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn elements_without_a_picker_attribute_are_not_pickers() {
        assert!(Pick::from_attrs(0, &attrs(&[("phx-click", "go")])).is_none());
        assert!(Pick::from_attrs(0, &BTreeMap::new()).is_none());
    }

    #[test]
    fn reads_an_open_picker() {
        let pick = Pick::from_attrs(
            3,
            &attrs(&[
                ("phx-pick-file", "chosen"),
                ("accept", "csv, .xlsx,,  "),
                ("phx-value-kind", "import"),
                ("class", "ignored"),
            ]),
        )
        .unwrap();
        assert_eq!(
            (pick.window, pick.event.as_str(), pick.save),
            (3, "chosen", false)
        );
        assert_eq!(pick.accept, ["csv", "xlsx"]);
        assert_eq!(pick.values, attrs(&[("kind", "import")]));
        assert_eq!(pick.default_name, None);
    }

    #[test]
    fn reads_a_save_picker() {
        let pick = Pick::from_attrs(
            0,
            &attrs(&[("phx-pick-save", "saved"), ("default-name", "out.csv")]),
        )
        .unwrap();
        assert!(pick.save && pick.accept.is_empty());
        assert_eq!(pick.event, "saved");
        assert_eq!(pick.default_name.as_deref(), Some("out.csv"));
    }

    #[test]
    fn open_wins_when_both_attributes_are_given() {
        let pick =
            Pick::from_attrs(0, &attrs(&[("phx-pick-save", "s"), ("phx-pick-file", "f")])).unwrap();
        assert_eq!((pick.event.as_str(), pick.save), ("f", false));
    }

    #[test]
    fn sends_the_path_with_the_values() {
        let h = harness();
        let pick = Pick::from_attrs(
            4,
            &attrs(&[("phx-pick-file", "chosen"), ("phx-value-id", "7")]),
        )
        .unwrap();
        pick.send("/tmp/a.csv".into());
        assert_eq!(
            h.events(),
            [json!({
                "type": "pick",
                "window": 4,
                "event": "chosen",
                "values": {"id": "7", "path": "/tmp/a.csv"},
            })]
        );
    }

    #[test]
    fn a_picker_answers_with_the_hooked_path_or_logs_that_it_cannot_open() {
        let h = harness();
        let pick = || Pick::from_attrs(1, &attrs(&[("phx-pick-save", "saved")])).unwrap();
        {
            let _hook = hook::set("/tmp/answer.txt");
            pick().open();
        }
        assert_eq!(
            h.events(),
            [json!({"type": "pick", "window": 1, "event": "saved",
                    "values": {"path": "/tmp/answer.txt"}})]
        );
        // With no hook and no app to open a dialog from, nothing is sent.
        if std::env::var_os("LEVAIN_PICK_FILE").is_none() {
            pick().open();
            assert!(h.events().is_empty());
        }
    }

    #[test]
    fn the_path_overrides_a_value_called_path() {
        let h = harness();
        let pick = Pick::from_attrs(
            0,
            &attrs(&[("phx-pick-save", "saved"), ("phx-value-path", "stale")]),
        )
        .unwrap();
        pick.send("/new".into());
        assert_eq!(h.events()[0]["values"], json!({"path": "/new"}));
    }
}

/// Builds `pick` events for `fixtures/events.json` through [`Pick::send`].
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use crate::bridge::testing::Harness;
    use serde_json::Value;

    pub(crate) fn event(h: &Harness, name: &str, a: &Value) -> Option<Value> {
        if name != "pick" {
            return None;
        }
        let attrs: BTreeMap<String, String> = a["attrs"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        let pick = Pick::from_attrs(a["window"].as_u64().unwrap() as u32, &attrs).unwrap();
        pick.send(a["path"].as_str().unwrap().into());
        let mut events = h.events();
        assert_eq!(events.len(), 1);
        Some(events.remove(0))
    }
}
