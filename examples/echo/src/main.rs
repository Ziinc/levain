//! A host with no language runtime: a counter written in Rust against the
//! C ABI, for levain's own end-to-end test.

use std::ffi::c_void;
use std::sync::mpsc;

use levain::levain_abi::{EventSink, HostApi};
use levain::{App, AppPaths, Runtime, WindowConfig};
use serde_json::{Value, json};

struct Echo;

impl Runtime for Echo {
    fn locate(&self, _: &AppPaths) -> Result<App, String> {
        Ok(App {
            root: ".".into(),
            name: "echo".into(),
            window: WindowConfig {
                title: Some("Echo".into()),
                width: 320.0,
                height: 200.0,
            },
        })
    }

    fn start(&self, _: App, api: &'static HostApi) -> Result<(), String> {
        std::thread::spawn(move || run(api));
        Ok(())
    }
}

extern "C" fn on_event(ctx: *mut c_void, json: *const u8, len: usize) {
    // SAFETY: `ctx` is the leaked sender `run` installed; the host passes a
    // valid buffer for the call.
    let (tx, bytes) = unsafe {
        (
            &*(ctx as *const mpsc::Sender<Vec<u8>>),
            std::slice::from_raw_parts(json, len),
        )
    };
    let _ = tx.send(bytes.to_vec());
}

fn render(api: &HostApi, count: i64) {
    let button =
        |event: &str, label: &str| json!({"t": "button", "a": {"phx-click": event}, "c": [label]});
    let frame = json!({"root": {
        "t": "div",
        "a": {"class": "size-full flex flex-col items-center justify-center gap-4"},
        "c": [
            {"t": "p", "c": [format!("Count: {count}")]},
            {"t": "div", "a": {"class": "flex gap-2"}, "c": [button("dec", "-"), button("inc", "+")]},
        ],
    }});
    let bytes = frame.to_string().into_bytes();
    (api.render)(api.ctx, 0, bytes.as_ptr(), bytes.len());
}

fn run(api: &'static HostApi) {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let ctx = Box::into_raw(Box::new(tx)) as *mut c_void;
    (api.set_event_sink)(
        api.ctx,
        EventSink {
            ctx,
            call: on_event,
        },
    );
    let mut count = 0;
    render(api, count);
    while let Ok(bytes) = rx.recv() {
        let Ok(event) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        if event["type"] != "click" {
            continue;
        }
        match event["event"].as_str() {
            Some("inc") => count += 1,
            Some("dec") => count -= 1,
            _ => continue,
        }
        render(api, count);
    }
}

fn main() {
    levain::run(Echo, tauri::generate_context!());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    static FRAMES: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    static SINK: Mutex<Option<EventSink>> = Mutex::new(None);

    extern "C" fn render_into_frames(_: *mut c_void, window: u32, json: *const u8, len: usize) {
        // SAFETY: `render` passes a buffer valid for `len` bytes for this call.
        let json = unsafe { std::slice::from_raw_parts(json, len) };
        let mut frame: Value = serde_json::from_slice(json).unwrap();
        frame["window"] = window.into();
        FRAMES.lock().unwrap().push(frame);
    }

    extern "C" fn keep_sink(_: *mut c_void, sink: EventSink) {
        *SINK.lock().unwrap() = Some(sink);
    }

    extern "C" fn accept(_: *mut c_void, _: *const u8, _: usize) -> bool {
        true
    }

    extern "C" fn ignore(_: *mut c_void, _: *const u8, _: usize) {}

    fn send(event: &str) {
        let sink = SINK
            .lock()
            .unwrap()
            .expect("the runtime installed its sink");
        (sink.call)(sink.ctx, event.as_ptr(), event.len());
    }

    /// The text of the frame's count label.
    fn count(frame: &Value) -> String {
        frame["root"]["c"][0]["c"][0].as_str().unwrap().to_owned()
    }

    fn wait_for_frames(n: usize) -> Vec<Value> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let frames = FRAMES.lock().unwrap().clone();
            if frames.len() >= n || Instant::now() > deadline {
                return frames;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_counter_counts_clicks_and_ignores_everything_else() {
        let app = Echo.locate(&AppPaths::default()).unwrap();
        assert_eq!(
            (app.name.as_str(), app.window.title.as_deref()),
            ("echo", Some("Echo"))
        );
        assert_eq!((app.window.width, app.window.height), (320.0, 200.0));

        let api: &'static HostApi = Box::leak(Box::new(HostApi {
            abi_version: levain::levain_abi::ABI_VERSION,
            ctx: std::ptr::null_mut(),
            render: render_into_frames,
            set_event_sink: keep_sink,
            check_components: accept,
            call: ignore,
        }));
        Echo.start(app, api).unwrap();

        let first = wait_for_frames(1);
        assert_eq!(count(&first[0]), "Count: 0");
        assert_eq!(first[0]["window"], 0);
        // The buttons send the app's events on click.
        let buttons = &first[0]["root"]["c"][1]["c"];
        assert_eq!(buttons[0]["a"]["phx-click"], "dec");
        assert_eq!(buttons[1]["a"]["phx-click"], "inc");

        let click =
            |event: &str| format!(r#"{{"type": "click", "window": 0, "event": "{event}"}}"#);
        send(&click("inc"));
        send(&click("inc"));
        // None of these is a click on `inc` or `dec`, so none renders.
        send("not json");
        send(r#"{"type": "change", "window": 0, "event": "inc"}"#);
        send(&click("other"));
        send(r#"{"type": "click", "window": 0}"#);
        send(&click("dec"));
        let frames = wait_for_frames(4);
        let counts: Vec<String> = frames.iter().map(count).collect();
        assert_eq!(counts, ["Count: 0", "Count: 1", "Count: 2", "Count: 1"]);
    }
}
