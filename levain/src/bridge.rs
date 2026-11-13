//! The host side of the host ↔ NIF ABI (see `levain_abi`).

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use levain_abi::{ABI_VERSION, EventSink, HostApi};

/// A render for one window, as sent by Elixir.
pub struct RenderMsg {
    pub window: u32,
    pub json: Vec<u8>,
}

struct Bridge {
    renders: async_channel::Sender<RenderMsg>,
    calls: async_channel::Sender<Vec<u8>>,
    sink: Mutex<Option<EventSink>>,
    /// Set when Elixir's component sets don't match: renders are ignored.
    refused: AtomicBool,
}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

/// What arrives from Elixir: renders, and platform calls (JSON).
pub struct Inbox {
    pub renders: async_channel::Receiver<RenderMsg>,
    pub calls: async_channel::Receiver<Vec<u8>>,
}

/// Creates the bridge and returns the [`HostApi`] to hand to the BEAM, and
/// the receivers renders and calls arrive on.
pub fn install() -> (&'static HostApi, Inbox) {
    let (tx, rx) = async_channel::unbounded();
    let (calls_tx, calls_rx) = async_channel::unbounded();
    let bridge = Bridge {
        renders: tx,
        calls: calls_tx,
        sink: Mutex::new(None),
        refused: AtomicBool::new(false),
    };
    assert!(BRIDGE.set(bridge).is_ok(), "bridge installed twice");
    let api = Box::leak(Box::new(HostApi {
        abi_version: ABI_VERSION,
        ctx: std::ptr::null_mut(),
        render,
        set_event_sink,
        check_components,
        call,
    }));
    let inbox = Inbox {
        renders: rx,
        calls: calls_rx,
    };
    (api, inbox)
}

extern "C" fn call(_ctx: *mut c_void, json: *const u8, len: usize) {
    // SAFETY: the NIF passes a buffer valid for `len` bytes for this call.
    let json = unsafe { std::slice::from_raw_parts(json, len) }.to_vec();
    if let Some(bridge) = BRIDGE.get() {
        let _ = bridge.calls.try_send(json);
    }
}

extern "C" fn render(_ctx: *mut c_void, window: u32, json: *const u8, len: usize) {
    // SAFETY: the NIF passes a buffer valid for `len` bytes for this call.
    let json = unsafe { std::slice::from_raw_parts(json, len) }.to_vec();
    if let Some(bridge) = BRIDGE.get().filter(|b| !b.refused.load(Ordering::Relaxed)) {
        let _ = bridge.renders.try_send(RenderMsg { window, json });
    }
}

extern "C" fn set_event_sink(_ctx: *mut c_void, sink: EventSink) {
    if let Some(bridge) = BRIDGE.get() {
        *bridge.sink.lock().unwrap() = Some(sink);
    }
}

extern "C" fn check_components(_ctx: *mut c_void, json: *const u8, len: usize) -> bool {
    // SAFETY: the NIF passes a buffer valid for `len` bytes for this call.
    let json = unsafe { std::slice::from_raw_parts(json, len) };
    let elixir: BTreeMap<String, String> = serde_json::from_slice(json).unwrap_or_default();
    let Some(refusal) = refusal(&crate::components::check(
        &crate::components::hashes(),
        &elixir,
    )) else {
        return true;
    };
    crate::log_line(&format!("baguette: {refusal}"));
    if let Some(bridge) = BRIDGE.get() {
        let frame = serde_json::json!({
            "root": {
                "t": "div",
                "a": {"class": "size-full flex items-center justify-center p-6 text-sm"},
                "c": [refusal],
            }
        });
        let json = frame.to_string().into_bytes();
        let _ = bridge.renders.try_send(RenderMsg { window: 0, json });
        bridge.refused.store(true, Ordering::Relaxed);
    }
    false
}

/// Why the host refuses Elixir's component sets, if it does. Sets the host
/// lacks are only logged: their tags draw as plain elements.
fn refusal(check: &crate::components::Check) -> Option<String> {
    for set in &check.missing {
        crate::log_line(&format!(
            "baguette: this host has no `{set}` component set; its tags draw as <div>s"
        ));
    }
    (!check.mismatched.is_empty()).then(|| {
        format!(
            "The app was compiled against other native components ({}) than this host has. \
             Rebuild the release so both come from the same manifests.",
            check.mismatched.join(", ")
        )
    })
}

/// Sends a UI event to Elixir. Dropped when no process is attached yet.
pub fn emit(event: &serde_json::Value) {
    let Some(sink) = BRIDGE.get().and_then(|b| *b.sink.lock().unwrap()) else {
        return;
    };
    let json = event.to_string();
    (sink.call)(sink.ctx, json.as_ptr(), json.len());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Check;

    #[test]
    fn refuses_only_mismatched_sets() {
        assert_eq!(refusal(&Check::default()), None);
        let missing = Check {
            missing: vec!["gpui-bootstrap".into()],
            ..Check::default()
        };
        assert_eq!(refusal(&missing), None);
        let mismatched = Check {
            mismatched: vec!["gpui-kit".into()],
            ..Check::default()
        };
        assert!(refusal(&mismatched).unwrap().contains("(gpui-kit)"));
    }

    #[test]
    fn accepts_the_hosts_own_hashes() {
        let json = serde_json::to_vec(&crate::components::hashes()).unwrap();
        assert!(check_components(
            std::ptr::null_mut(),
            json.as_ptr(),
            json.len()
        ));
    }
}
