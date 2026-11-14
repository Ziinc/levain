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
