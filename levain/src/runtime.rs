//! The one seam between levain and a language: a [`Runtime`] finds its app
//! and starts it; from then on it talks to the host only through the C ABI.

use std::path::PathBuf;

/// Runs one language's apps on levain.
pub trait Runtime: Send + 'static {
    /// The app next to the executable, in the bundle's resources, or
    /// unpacked from the host library on mobile ([`crate::mobile::unpack`]).
    fn locate(&self, paths: &AppPaths) -> Result<App, String>;

    /// Starts the app on its own thread. It calls `api.render`, `api.call`
    /// and `api.set_event_sink`, never GPUI directly.
    fn start(&self, app: App, api: &'static levain_abi::HostApi) -> Result<(), String>;
}

/// Where the host may find its app.
#[derive(Debug, Default, Clone)]
pub struct AppPaths {
    /// The running executable.
    pub exe: Option<PathBuf>,
    /// Tauri's resource directory (desktop).
    pub resources: Option<PathBuf>,
    /// The app's local data directory, which an app embedded in the host
    /// library unpacks into (mobile).
    pub data: Option<PathBuf>,
}

/// An app a runtime located.
#[derive(Debug, Clone)]
pub struct App {
    pub root: PathBuf,
    pub name: String,
    pub window: WindowConfig,
}

/// The main window, from the app's `app.json` (baguette's `baguette.json`).
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    pub title: Option<String>,
    pub width: f64,
    pub height: f64,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: None,
            width: 480.0,
            height: 360.0,
        }
    }
}

impl WindowConfig {
    /// The `window` of an `app.json`; the default when it is missing or
    /// malformed.
    pub fn from_app_json(json: &[u8]) -> Self {
        #[derive(serde::Deserialize)]
        struct Config {
            window: WindowConfig,
        }
        serde_json::from_slice::<Config>(json)
            .map(|config| config.window)
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_from_app_json() {
        let json = br#"{"window": {"title": "Counter", "width": 320}}"#;
        assert_eq!(
            WindowConfig::from_app_json(json),
            WindowConfig {
                title: Some("Counter".into()),
                width: 320.0,
                height: 360.0
            }
        );
    }

    #[test]
    fn default_window_without_app_json() {
        assert_eq!(WindowConfig::from_app_json(b""), WindowConfig::default());
        assert_eq!(WindowConfig::from_app_json(b"{}"), WindowConfig::default());
    }
}
