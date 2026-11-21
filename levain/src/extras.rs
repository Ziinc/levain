//! What an app's `app.json` asks of a desktop host beyond its window:
//! its own URL schemes (`deep_link`), one instance at a time
//! (`single_instance`) and updates (`updater`). Each is the config Tauri's
//! plugin takes, under `plugins` in `tauri.conf.json`.

use serde::Deserialize;
use serde_json::Value;

use crate::bridge;
use crate::platform::Request;

// Phones get these from their projects' manifests instead.
#[cfg_attr(mobile, allow(dead_code))]
#[derive(Deserialize, Default, Debug, PartialEq)]
pub struct Extras {
    /// `{"desktop": {"schemes": ["myapp"]}}`.
    #[serde(default)]
    pub deep_link: Option<Value>,
    #[serde(default)]
    pub single_instance: bool,
    /// `{"pubkey": "...", "endpoints": ["https://..."]}`. Releases are
    /// signed with the matching private key, which only the developer has
    /// (`TAURI_SIGNING_PRIVATE_KEY` when bundling).
    #[serde(default)]
    pub updater: Option<Value>,
}

#[cfg_attr(mobile, allow(dead_code))]
impl Extras {
    /// Reads them from `app.json` in `root`; none when it has none.
    pub fn read(root: &std::path::Path) -> Self {
        std::fs::read(root.join("app.json"))
            .ok()
            .and_then(|json| serde_json::from_slice(&json).ok())
            .unwrap_or_default()
    }

    /// Puts the plugins' config where Tauri reads it.
    pub fn configure(&self, config: &mut tauri::Config) {
        if let Some(deep_link) = &self.deep_link {
            config
                .plugins
                .0
                .insert("deep-link".into(), deep_link.clone());
        }
        if let Some(updater) = &self.updater {
            config.plugins.0.insert("updater".into(), updater.clone());
        }
    }

    /// Registers the plugins asked for; the single-instance one first, as
    /// it must be.
    #[cfg(desktop)]
    pub fn plugins<R: tauri::Runtime>(&self, builder: tauri::Builder<R>) -> tauri::Builder<R> {
        let mut builder = builder;
        if self.single_instance {
            builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
                use tauri::Manager;
                if let Some(main) = app.get_window(&crate::windows::label(0)) {
                    let _ = main.set_focus();
                }
                bridge::emit(&crate::events::instance(&args, &cwd));
            }));
        }
        if self.deep_link.is_some() {
            builder = builder.plugin(tauri_plugin_deep_link::init());
        }
        if self.updater.is_some() {
            builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
        }
        builder
    }

    /// Sends the app its deep links. On Linux and Windows the schemes are
    /// registered with the desktop at startup, as installers would.
    #[cfg(desktop)]
    pub fn setup<R: tauri::Runtime>(&self, app: &tauri::AppHandle<R>) {
        if self.deep_link.is_none() {
            return;
        }
        use tauri_plugin_deep_link::DeepLinkExt;
        let links = app.deep_link();
        #[cfg(any(windows, target_os = "linux"))]
        if let Err(error) = links.register_all() {
            crate::log_line(&format!(
                "levain: could not register the app's URL schemes: {error}"
            ));
        }
        links.on_open_url(|event| {
            bridge::emit(&crate::events::opened(
                event.urls().iter().map(|u| u.as_str()),
            ));
        });
    }
}

/// The update calls.
pub const CALLS: [&str; 2] = ["update.check", "update.install"];

pub fn handles(call: &str) -> bool {
    CALLS.contains(&call)
}

/// Checks for an update, or installs it, and replies when done: `null` for
/// none, else `{version, current_version, date, notes}`. After installing,
/// the app restarts itself with `app.restart` when it is ready to.
pub fn handle<R: tauri::Runtime>(request: Request, app: &tauri::AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let outcome = update(&app, request.call == "update.install").await;
        bridge::emit(&crate::platform::reply_for(&request, outcome));
    });
}

#[cfg(desktop)]
async fn update<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    install: bool,
) -> Result<Value, String> {
    use tauri::Manager;
    use tauri_plugin_updater::UpdaterExt;
    if app.try_state::<tauri_plugin_updater::Updater>().is_none() {
        return Err("the app declares no updater".into());
    }
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    let Some(update) = update else {
        return Ok(Value::Null);
    };
    let found = serde_json::json!({
        "version": update.version,
        "current_version": update.current_version,
        "date": update.date.map(|d| d.to_string()),
        "notes": update.body,
    });
    if install {
        update
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(found)
}

#[cfg(mobile)]
async fn update<R: tauri::Runtime>(_: &tauri::AppHandle<R>, _: bool) -> Result<Value, String> {
    Err("phones update through their app stores".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extras_are_read_from_app_json_and_default_to_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Extras::read(dir.path()), Extras::default());
        std::fs::write(
            dir.path().join("app.json"),
            r#"{"app": "x:main", "single_instance": true,
                "deep_link": {"desktop": {"schemes": ["jobs"]}},
                "updater": {"pubkey": "PUB", "endpoints": ["https://example.com/{{target}}/{{current_version}}"]}}"#,
        )
        .unwrap();
        let extras = Extras::read(dir.path());
        assert!(extras.single_instance);
        let mut config =
            tauri::test::mock_context::<tauri::test::MockRuntime, _>(tauri::test::noop_assets())
                .config()
                .clone();
        extras.configure(&mut config);
        assert_eq!(
            config.plugins.0["deep-link"]["desktop"]["schemes"][0],
            "jobs"
        );
        assert_eq!(config.plugins.0["updater"]["pubkey"], "PUB");
    }

    #[test]
    fn checking_for_updates_without_an_updater_says_so() {
        let h = crate::bridge::testing::harness();
        let app = tauri::test::mock_app();
        handle(
            Request {
                call: "update.check".into(),
                reference: 9,
                args: Value::Null,
            },
            app.handle(),
        );
        let mut events = Vec::new();
        for _ in 0..200 {
            events.extend(h.events());
            if !events.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            events,
            [json!({"type": "reply", "ref": 9, "result": null,
                    "error": "update.check: the app declares no updater"})]
        );
    }
}
