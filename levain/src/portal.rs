//! The desktop's colour scheme, from the XDG desktop portal (Linux).
//!
//! TAO is built without its own portal lookup, which blocks the main thread
//! 5 s per call when the portal never answers. This reads the portal's
//! `color-scheme` on a thread of its own and follows its changes. Until the
//! portal answers, and where there is none, GPUI's appearance (GTK's theme)
//! stands.

use tauri_plugin_gpui::gpui::{self, Global, WindowAppearance};

/// The portal's colour scheme, once it has answered.
#[derive(Default)]
struct Scheme(Option<WindowAppearance>);

impl Global for Scheme {}

/// The app's appearance: the portal's once it has answered, else GPUI's.
pub fn appearance(cx: &gpui::App) -> WindowAppearance {
    cx.try_global::<Scheme>()
        .and_then(|scheme| scheme.0)
        .unwrap_or_else(|| cx.window_appearance())
}

/// Starts following the portal. A new colour scheme is stored on the UI
/// thread and sent to every window's view.
pub fn watch(cx: &mut gpui::App) {
    let (tx, rx) = async_channel::unbounded::<WindowAppearance>();
    #[cfg(target_os = "linux")]
    let _ = std::thread::Builder::new()
        .name("baguette-portal".into())
        .spawn(move || {
            if let Err(error) = futures_lite::future::block_on(follow(tx)) {
                crate::log_line(&format!(
                    "baguette: no colour scheme from the portal: {error}"
                ));
            }
        });
    #[cfg(not(target_os = "linux"))]
    drop(tx);
    cx.spawn(async move |cx| {
        while let Ok(appearance) = rx.recv().await {
            cx.update(|cx| {
                let changed = cx.try_global::<Scheme>().and_then(|s| s.0) != Some(appearance);
                cx.set_global(Scheme(Some(appearance)));
                if changed {
                    crate::windows::appearance_changed(cx);
                }
            });
        }
    })
    .detach();
}

#[cfg(target_os = "linux")]
async fn follow(tx: async_channel::Sender<WindowAppearance>) -> ashpd::Result<()> {
    use ashpd::desktop::settings::Settings;
    use futures_lite::StreamExt;

    let settings = Settings::new().await?;
    let changes = settings.receive_color_scheme_changed().await?;
    if tx
        .send(scheme(settings.color_scheme().await?))
        .await
        .is_err()
    {
        return Ok(());
    }
    let mut changes = std::pin::pin!(changes);
    while let Some(change) = changes.next().await {
        if tx.send(scheme(change)).await.is_err() {
            break;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn scheme(scheme: ashpd::desktop::settings::ColorScheme) -> WindowAppearance {
    use ashpd::desktop::settings::ColorScheme;
    match scheme {
        ColorScheme::PreferDark => WindowAppearance::Dark,
        ColorScheme::PreferLight | ColorScheme::NoPreference => WindowAppearance::Light,
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use ashpd::desktop::settings::ColorScheme;

    #[test]
    fn dark_only_when_asked_for() {
        assert_eq!(scheme(ColorScheme::PreferDark), WindowAppearance::Dark);
        assert_eq!(scheme(ColorScheme::PreferLight), WindowAppearance::Light);
        assert_eq!(scheme(ColorScheme::NoPreference), WindowAppearance::Light);
    }
}
