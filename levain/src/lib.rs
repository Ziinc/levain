//! levain, the shared native host of the Boulangerie libraries. Tauri owns
//! the event loop and windows and GPUI renders into them; a language
//! [`Runtime`] starts the app on its own thread and drives the host through
//! the C ABI (`levain_abi`), with JSON node trees, patches, events and
//! platform calls.
//!
//! A language host is an executable (desktop) or library (Android, iOS)
//! whose entry point is [`run`].

mod animation;
mod bridge;
pub mod components;
pub mod events;
mod keymap;
mod list;
#[cfg_attr(not(mobile), allow(dead_code))]
#[cfg(any(mobile, test))]
pub mod mobile;
mod paint;
mod patch;
mod pick;
mod platform;
#[cfg(feature = "playwright")]
mod playwright;
mod portal;
mod runtime;
mod style;
mod style_gen;
#[cfg(test)]
mod testing;
mod view;
mod windows;

pub use levain_abi;
pub use runtime::{App, AppPaths, Runtime, WindowConfig};
pub use tauri;

use crate::view::{Frame, Node};

pub(crate) fn notice(text: String) -> Frame {
    Frame {
        root: Some(Node::Element {
            tag: "div".into(),
            attrs: [(
                "class".into(),
                "size-full flex items-center justify-center p-6 text-sm".into(),
            )]
            .into(),
            children: vec![Node::Text(text)],
        }),
        ..Frame::default()
    }
}

/// Runs the host: `context` is the language host's own
/// `tauri::generate_context!()`, so its `tauri.conf.json` names the app.
///
/// On desktop, `--manifest [set]` prints the registered native components,
/// all or one set's, and exits.
pub fn run<R: Runtime>(runtime: R, context: tauri::Context<tauri::Wry>) {
    #[cfg(desktop)]
    if let Some(manifest) = manifest_arg(&std::env::args().collect::<Vec<_>>()) {
        match manifest {
            Ok(manifest) => println!("{manifest:#}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1)
            }
        }
        return;
    }
    #[cfg(target_os = "android")]
    {
        android_logger::init_once(
            android_logger::Config::default()
                .with_max_level(log::LevelFilter::Info)
                .with_tag("levain"),
        );
        mobile::log_stdio("app");
    }
    let (host, inbox) = bridge::install();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init());
    // Menus are desktop only.
    #[cfg(desktop)]
    let builder = builder.on_menu_event(|_, event| platform::on_menu_event(event));
    builder
        .setup(move |app| {
            pick::install(app.handle().clone());
            use tauri::Manager;
            let paths = AppPaths {
                exe: std::env::current_exe().ok(),
                resources: app.path().resource_dir().ok(),
                data: app.path().app_local_data_dir().ok(),
            };
            let located = runtime.locate(&paths);
            let handle = app.handle().clone();
            let handle2 = handle.clone();
            tauri_plugin_gpui::init_with(
                app,
                tauri_plugin_gpui::GpuiConfig::new()
                    // gpui-kit's icons (a checkbox's tick, an input's clear button).
                    // and every lucide icon, for `<icon name="...">`.
                    .assets(gpui_kit::assets::AllAssets)
                    .on_launch(move |cx| {
                        let open = move |id, title: &str, size| {
                            windows::open(&handle, id, title, size, Frame::default())
                        };
                        portal::watch(cx);
                        launch(cx, inbox, handle2, open)
                    }),
            )?;
            let (title, size, frame) = window_zero(&located);
            windows::open(app.handle(), 0, &title, size, frame)?;

            if let Ok(found) = located {
                runtime.start(found, host)?;
            }
            Ok(())
        })
        .run(context)
        .expect("error while running the levain host");
}

/// Starts GPUI's side of the host once its app is up: gpui-kit's components
/// and the loops that show the app's renders and
/// run its platform calls on the UI thread.
fn launch<R: tauri::Runtime>(
    cx: &mut tauri_plugin_gpui::gpui::App,
    inbox: bridge::Inbox,
    app: tauri::AppHandle<R>,
    open: impl Fn(u32, &str, (f64, f64)) -> Result<(), Box<dyn std::error::Error>> + 'static,
) {
    // gpui-kit's components: text inputs and checkboxes.
    gpui_kit::init(cx);
    #[cfg(feature = "playwright")]
    playwright::start(cx);
    let bridge::Inbox { renders, calls } = inbox;
    cx.spawn(async move |cx| {
        let mut first = true;
        while let Ok(msg) = renders.recv().await {
            if std::mem::take(&mut first) {
                log_line("levain: first frame");
            }
            cx.update(|cx| windows::route(msg, cx));
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Ok(json) = calls.recv().await {
            cx.update(|cx| platform::handle(&json, &app, &open, cx));
        }
    })
    .detach();
}

/// Window 0's title, size and first frame: the app's, or a notice of why
/// there is none.
fn window_zero(located: &Result<App, String>) -> (String, (f64, f64), Frame) {
    match located {
        Ok(app) => (
            app.window.title.clone().unwrap_or(app.name.clone()),
            (app.window.width, app.window.height),
            Frame::default(),
        ),
        Err(error) => {
            let window = WindowConfig::default();
            (
                "levain".into(),
                (window.width, window.height),
                notice(format!("No app to run: {error}")),
            )
        }
    }
}

/// What `--manifest [set]` prints, when it is among `args`.
#[cfg_attr(mobile, allow(dead_code))]
fn manifest_arg(args: &[String]) -> Option<Result<serde_json::Value, String>> {
    let at = args.iter().position(|arg| arg == "--manifest")?;
    Some(match args.get(at + 1) {
        Some(set) => components::manifest_of(set)
            .ok_or_else(|| format!("no component set `{set}` in this host")),
        None => Ok(components::sets()
            .iter()
            .map(|set| components::manifest(set, components::Versions::BUILT))
            .collect()),
    })
}

/// A line for the platform log: stderr on desktop and iOS, logcat on Android.
pub fn log_line(line: &str) {
    #[cfg(target_os = "android")]
    log::info!("{line}");
    #[cfg(not(target_os = "android"))]
    eprintln!("{line}");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn manifest_only_when_asked() {
        assert!(manifest_arg(&args(&["host"])).is_none());
    }

    #[test]
    fn manifest_of_every_set_or_one() {
        let all = manifest_arg(&args(&["host", "--manifest"]))
            .unwrap()
            .unwrap();
        assert_eq!(all.as_array().unwrap().len(), components::sets().len());
        let kit = manifest_arg(&args(&["host", "--manifest", "gpui-kit"]))
            .unwrap()
            .unwrap();
        assert_eq!(Some(kit), components::manifest_of("gpui-kit"));
        assert!(
            manifest_arg(&args(&["host", "--manifest", "nope"]))
                .unwrap()
                .is_err()
        );
    }

    #[gpui_kit::test]
    fn launching_shows_the_apps_renders_and_runs_its_calls(cx: &mut gpui_kit::TestAppContext) {
        use crate::view::Node;
        let h = bridge::testing::harness();
        let app = tauri::test::mock_app();
        crate::testing::window(cx, 0);
        h.events();
        let inbox = bridge::Inbox {
            renders: h.inbox.renders.clone(),
            calls: h.inbox.calls.clone(),
        };
        let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let log = opened.clone();
        cx.update(|cx| {
            launch(
                cx,
                inbox,
                app.handle().clone(),
                move |id, title: &str, size| {
                    log.borrow_mut().push((id, title.to_owned(), size));
                    Ok(())
                },
            )
        });
        // The app renders window 0 and asks for window 4.
        let ctx = h.api.ctx;
        let frame = br#"{"root": "from the app"}"#;
        (h.api.render)(ctx, 0, frame.as_ptr(), frame.len());
        let call = br#"{"call": "window.open", "ref": 3, "args": {"window": 4, "title": "Four"}}"#;
        (h.api.call)(ctx, call.as_ptr(), call.len());
        cx.run_until_parked();
        let shown =
            cx.update(|cx| windows::root_of(0, cx).map(|root| root.read(cx).frame.root.clone()));
        assert_eq!(shown, Some(Some(Node::Text("from the app".into()))));
        assert_eq!(*opened.borrow(), [(4, "Four".to_owned(), (800., 600.))]);
        assert_eq!(
            h.events().last(),
            Some(&serde_json::json!({"type": "reply", "ref": 3, "result": null}))
        );
    }

    #[test]
    fn window_zero_is_the_apps_or_a_notice() {
        let app = App {
            root: "/app".into(),
            name: "counter".into(),
            window: WindowConfig {
                title: None,
                width: 320.0,
                height: 200.0,
            },
        };
        let (title, size, frame) = window_zero(&Ok(app));
        assert_eq!((title.as_str(), size), ("counter", (320.0, 200.0)));
        assert!(frame.root.is_none());
        let (_, _, frame) = window_zero(&Err("missing".into()));
        assert!(frame.root.is_some());
    }
}
