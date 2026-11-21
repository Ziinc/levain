//! Native dialogs as platform calls: messages, confirmations, and file
//! open and save dialogs. Their replies come once the user answers.
//!
//! Test hook: with `LEVAIN_PICK_FILE` set, open and save dialogs answer with
//! that path at once, as `phx-pick-file` does (`pick.rs`).

use serde::Deserialize;
use serde_json::{Value, json};

use crate::bridge;
use crate::platform::Request;

#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub name: String,
    pub extensions: Vec<String>,
}

/// The arguments of every dialog call; each reads the ones it takes.
#[derive(Deserialize, Debug, Default, PartialEq)]
#[serde(default)]
pub struct DialogArgs {
    pub title: Option<String>,
    pub message: Option<String>,
    /// `info` (the default), `warning` or `error`.
    pub kind: Option<String>,
    /// A confirmation's button texts.
    pub ok: Option<String>,
    pub cancel: Option<String>,
    pub multiple: bool,
    pub directory: bool,
    pub filters: Vec<Filter>,
    pub default_name: Option<String>,
    pub default_path: Option<String>,
}

/// The dialog calls.
pub const CALLS: [&str; 4] = [
    "dialog.message",
    "dialog.confirm",
    "dialog.open",
    "dialog.save",
];

/// Whether `call` is one of these.
pub fn handles(call: &str) -> bool {
    CALLS.contains(&call)
}

/// Reads `request`'s arguments: an error for ones the call cannot take.
pub fn args(request: &Request) -> Result<DialogArgs, String> {
    let args: DialogArgs = if request.args.is_null() {
        DialogArgs::default()
    } else {
        serde_json::from_value(request.args.clone()).map_err(|e| e.to_string())?
    };
    if matches!(request.call.as_str(), "dialog.message" | "dialog.confirm")
        && args.message.is_none()
    {
        return Err("no `message`".into());
    }
    if let Some(kind) = &args.kind
        && !matches!(kind.as_str(), "info" | "warning" | "error")
    {
        return Err(format!("no dialog kind `{kind}`"));
    }
    Ok(args)
}

/// A file dialog's answer as the app gets it: a path, a list of them when
/// `multiple`, or `null` for a cancel.
pub fn answer(paths: Option<Vec<String>>, multiple: bool) -> Value {
    match (paths, multiple) {
        (None, _) => Value::Null,
        (Some(paths), true) => json!(paths),
        (Some(paths), false) => paths.into_iter().next().map_or(Value::Null, Value::from),
    }
}

/// Shows the dialog; `reply` gets its answer when the user gives it.
pub fn show<R: tauri::Runtime>(
    request: &Request,
    app: &tauri::AppHandle<R>,
    reply: impl FnOnce(Result<Value, String>) + Send + 'static,
) {
    let args = match args(request) {
        Ok(args) => args,
        Err(error) => return reply(Err(error)),
    };
    let file = matches!(request.call.as_str(), "dialog.open" | "dialog.save");
    if file && let Some(path) = std::env::var_os("LEVAIN_PICK_FILE") {
        let path = path.to_string_lossy().into_owned();
        return reply(Ok(answer(Some(vec![path]), args.multiple)));
    }
    use tauri::Manager;
    if app.try_state::<tauri_plugin_dialog::Dialog<R>>().is_none() {
        return reply(Err("no dialog plugin in this host".into()));
    }
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let dialogs = app.dialog();
    match request.call.as_str() {
        "dialog.message" | "dialog.confirm" => {
            let mut dialog = dialogs.message(args.message.unwrap_or_default()).kind(
                match args.kind.as_deref() {
                    Some("warning") => MessageDialogKind::Warning,
                    Some("error") => MessageDialogKind::Error,
                    _ => MessageDialogKind::Info,
                },
            );
            if let Some(title) = args.title {
                dialog = dialog.title(title);
            }
            let confirm = request.call == "dialog.confirm";
            dialog = dialog.buttons(match (confirm, args.ok, args.cancel) {
                (false, Some(ok), _) => MessageDialogButtons::OkCustom(ok),
                (false, None, _) => MessageDialogButtons::Ok,
                (true, ok, cancel) => MessageDialogButtons::OkCancelCustom(
                    ok.unwrap_or_else(|| "OK".into()),
                    cancel.unwrap_or_else(|| "Cancel".into()),
                ),
            });
            dialog.show(move |ok| reply(Ok(if confirm { json!(ok) } else { Value::Null })));
        }
        _ => {
            let mut dialog = dialogs.file();
            for filter in &args.filters {
                let extensions: Vec<&str> = filter.extensions.iter().map(String::as_str).collect();
                dialog = dialog.add_filter(&filter.name, &extensions);
            }
            if let Some(title) = &args.title {
                dialog = dialog.set_title(title);
            }
            if let Some(name) = &args.default_name {
                dialog = dialog.set_file_name(name);
            }
            if let Some(path) = &args.default_path {
                dialog = dialog.set_directory(path);
            }
            let multiple = args.multiple;
            let paths = |paths: Option<Vec<tauri_plugin_dialog::FilePath>>| {
                paths.map(|paths| paths.into_iter().map(|p| p.to_string()).collect())
            };
            let one = |path: Option<tauri_plugin_dialog::FilePath>| path.map(|p| vec![p]);
            match (request.call.as_str(), args.directory, multiple) {
                ("dialog.save", ..) => {
                    dialog.save_file(move |p| reply(Ok(answer(paths(one(p)), multiple))))
                }
                (_, false, false) => {
                    dialog.pick_file(move |p| reply(Ok(answer(paths(one(p)), multiple))))
                }
                #[cfg(desktop)]
                (_, true, false) => {
                    dialog.pick_folder(move |p| reply(Ok(answer(paths(one(p)), multiple))))
                }
                (_, false, true) => dialog.pick_files(move |p| reply(Ok(answer(paths(p), true)))),
                #[cfg(desktop)]
                (_, true, true) => dialog.pick_folders(move |p| reply(Ok(answer(paths(p), true)))),
                #[cfg(mobile)]
                (_, true, _) => reply(Err("folder dialogs are desktop only".into())),
            }
        }
    }
}

/// Runs a dialog call, sending its reply when the user answers.
pub fn handle<R: tauri::Runtime>(request: Request, app: &tauri::AppHandle<R>) {
    let reference = request.reference;
    let call = request.call.clone();
    show(&request, app, move |outcome| {
        bridge::emit(&crate::platform::reply_for(
            &Request {
                call,
                reference,
                args: Value::Null,
            },
            outcome,
        ));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(call: &str, args: Value) -> Request {
        Request {
            call: call.into(),
            reference: 1,
            args,
        }
    }

    #[test]
    fn messages_need_a_message_and_a_known_kind() {
        assert_eq!(
            args(&request("dialog.message", json!({}))),
            Err("no `message`".into())
        );
        assert_eq!(
            args(&request(
                "dialog.confirm",
                json!({"message": "Delete?", "kind": "loud"})
            )),
            Err("no dialog kind `loud`".into())
        );
        let read = args(&request(
            "dialog.confirm",
            json!({"message": "Delete?", "kind": "warning", "ok": "Delete"}),
        ))
        .unwrap();
        assert_eq!(read.ok.as_deref(), Some("Delete"));
    }

    #[test]
    fn file_dialogs_take_filters_and_need_nothing() {
        let read = args(&request(
            "dialog.open",
            json!({"multiple": true, "filters": [{"name": "Images", "extensions": ["png", "jpg"]}]}),
        ))
        .unwrap();
        assert!(read.multiple);
        assert_eq!(read.filters[0].extensions, ["png", "jpg"]);
        assert_eq!(
            args(&request("dialog.save", Value::Null)),
            Ok(DialogArgs::default())
        );
        assert!(
            args(&request("dialog.open", json!({"filters": [{"name": "x"}]})))
                .unwrap_err()
                .contains("extensions")
        );
    }

    #[test]
    fn answers_are_a_path_paths_or_null() {
        let paths = || Some(vec!["/a".to_owned(), "/b".to_owned()]);
        assert_eq!(answer(paths(), false), json!("/a"));
        assert_eq!(answer(paths(), true), json!(["/a", "/b"]));
        assert_eq!(answer(None, true), Value::Null);
        assert_eq!(answer(Some(vec![]), false), Value::Null);
    }

    #[test]
    fn a_host_without_the_plugin_says_so() {
        let app = tauri::test::mock_app();
        let (tx, rx) = std::sync::mpsc::channel();
        show(
            &request("dialog.message", json!({"message": "hi"})),
            app.handle(),
            move |outcome| tx.send(outcome).unwrap(),
        );
        assert_eq!(
            rx.recv().unwrap(),
            Err("no dialog plugin in this host".into())
        );
    }
}
