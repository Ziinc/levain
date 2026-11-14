//! The host side of the host ↔ runtime ABI (see `levain_abi`).

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use levain_abi::{ABI_VERSION, EventSink, HostApi};

/// A render for one window, as sent by the app.
pub struct RenderMsg {
    pub window: u32,
    pub json: Vec<u8>,
}

struct Bridge {
    renders: async_channel::Sender<RenderMsg>,
    calls: async_channel::Sender<Vec<u8>>,
    sink: Mutex<Option<EventSink>>,
    /// Set when the app's component sets don't match: renders are ignored.
    refused: AtomicBool,
}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

/// What arrives from the app: renders, and platform calls (JSON).
pub struct Inbox {
    pub renders: async_channel::Receiver<RenderMsg>,
    pub calls: async_channel::Receiver<Vec<u8>>,
}

/// Creates the bridge and returns the [`HostApi`] to hand to the runtime, and
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
    // SAFETY: the runtime passes a buffer valid for `len` bytes for this call.
    let json = unsafe { std::slice::from_raw_parts(json, len) }.to_vec();
    if let Some(bridge) = BRIDGE.get() {
        let _ = bridge.calls.try_send(json);
    }
}

extern "C" fn render(_ctx: *mut c_void, window: u32, json: *const u8, len: usize) {
    // SAFETY: the runtime passes a buffer valid for `len` bytes for this call.
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
    // SAFETY: the runtime passes a buffer valid for `len` bytes for this call.
    let json = unsafe { std::slice::from_raw_parts(json, len) };
    let runtime: BTreeMap<String, String> = serde_json::from_slice(json).unwrap_or_default();
    let Some(refusal) = refusal(&crate::components::check(
        &crate::components::hashes(),
        &runtime,
    )) else {
        return true;
    };
    crate::log_line(&format!("levain: {refusal}"));
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

/// Why the host refuses the app's component sets, if it does. Sets the host
/// lacks are only logged: their tags draw as plain elements.
fn refusal(check: &crate::components::Check) -> Option<String> {
    for set in &check.missing {
        crate::log_line(&format!(
            "levain: this host has no `{set}` component set; its tags draw as <div>s"
        ));
    }
    (!check.mismatched.is_empty()).then(|| {
        format!(
            "The app was compiled against other native components ({}) than this host has. \
             Rebuild the app so both come from the same manifests.",
            check.mismatched.join(", ")
        )
    })
}

/// Sends a UI event to the app. Dropped when no process is attached yet.
pub fn emit(event: &serde_json::Value) {
    let Some(sink) = BRIDGE.get().and_then(|b| *b.sink.lock().unwrap()) else {
        return;
    };
    let json = event.to_string();
    (sink.call)(sink.ctx, json.as_ptr(), json.len());
}

/// A process-wide bridge for tests: `BRIDGE` can be installed once, so every
/// test that sends through it shares this one, one at a time.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    static SHARED: OnceLock<(&'static HostApi, Inbox)> = OnceLock::new();
    static LOCK: Mutex<()> = Mutex::new(());
    static EVENTS: Mutex<Vec<serde_json::Value>> = Mutex::new(Vec::new());

    extern "C" fn collect(_ctx: *mut c_void, json: *const u8, len: usize) {
        // SAFETY: `emit` passes a buffer valid for `len` bytes for this call.
        let json = unsafe { std::slice::from_raw_parts(json, len) };
        EVENTS
            .lock()
            .unwrap()
            .push(serde_json::from_slice(json).unwrap());
    }

    /// The sink that collects events for [`Harness::events`].
    pub fn sink() -> EventSink {
        EventSink {
            ctx: std::ptr::null_mut(),
            call: collect,
        }
    }

    /// Holds the bridge for one test, with its inbox empty, its refusal
    /// cleared and an event sink that collects what the host emits.
    pub struct Harness {
        _guard: std::sync::MutexGuard<'static, ()>,
        pub api: &'static HostApi,
        pub inbox: &'static Inbox,
    }

    pub fn harness() -> Harness {
        let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let (api, inbox) = SHARED.get_or_init(install);
        while inbox.renders.try_recv().is_ok() {}
        while inbox.calls.try_recv().is_ok() {}
        let bridge = BRIDGE.get().unwrap();
        bridge.refused.store(false, Ordering::Relaxed);
        *bridge.sink.lock().unwrap() = Some(sink());
        EVENTS.lock().unwrap().clear();
        Harness {
            _guard: guard,
            api,
            inbox,
        }
    }

    impl Harness {
        /// What the host emitted since the last call.
        pub fn events(&self) -> Vec<serde_json::Value> {
            std::mem::take(&mut *EVENTS.lock().unwrap())
        }

        /// Detaches the runtime: events are dropped again.
        pub fn detach(&self) {
            *BRIDGE.get().unwrap().sink.lock().unwrap() = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::harness;
    use super::*;
    use crate::components::Check;
    use serde_json::json;

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

    #[test]
    fn hands_out_its_abi_version_and_installs_once() {
        let h = harness();
        assert_eq!(h.api.abi_version, ABI_VERSION);
        assert!(h.api.ctx.is_null());
        let second = std::panic::catch_unwind(install);
        assert!(second.is_err(), "a second install must panic");
    }

    #[test]
    fn renders_and_calls_reach_the_inbox() {
        let h = harness();
        let ctx = h.api.ctx;
        (h.api.render)(ctx, 2, br#"{"root":null}"#.as_ptr(), 13);
        (h.api.call)(ctx, br#"{"call":"x"}"#.as_ptr(), 12);
        let msg = h.inbox.renders.try_recv().unwrap();
        assert_eq!(
            (msg.window, msg.json.as_slice()),
            (2, &br#"{"root":null}"#[..])
        );
        assert!(h.inbox.renders.try_recv().is_err());
        assert_eq!(h.inbox.calls.try_recv().unwrap(), br#"{"call":"x"}"#);
        // The two channels are separate.
        assert!(h.inbox.calls.try_recv().is_err());
    }

    #[test]
    fn events_go_to_the_latest_sink_and_are_dropped_without_one() {
        let h = harness();
        emit(&json!({"type": "resync", "window": 1}));
        assert_eq!(h.events(), [json!({"type": "resync", "window": 1})]);

        h.detach();
        emit(&json!({"type": "resync", "window": 2}));
        assert!(h.events().is_empty());

        // `set_event_sink` through the ABI attaches one again.
        (h.api.set_event_sink)(h.api.ctx, super::testing::sink());
        emit(&json!({"type": "menu", "window": 0, "event": "quit"}));
        assert_eq!(h.events().len(), 1);
    }

    #[test]
    fn a_mismatch_shows_why_and_ignores_later_renders() {
        let h = harness();
        let hashes = crate::components::hashes();
        let Some(set) = hashes.keys().next().cloned() else {
            return;
        };
        let wrong = serde_json::to_vec(&json!({ set.clone(): "not-the-hash" })).unwrap();
        assert!(!(h.api.check_components)(
            h.api.ctx,
            wrong.as_ptr(),
            wrong.len()
        ));

        let msg = h.inbox.renders.try_recv().expect("a refusal frame");
        assert_eq!(msg.window, 0);
        let frame: serde_json::Value = serde_json::from_slice(&msg.json).unwrap();
        assert_eq!(frame["root"]["t"], "div");
        let text = frame["root"]["c"][0].as_str().unwrap();
        assert!(text.contains("compiled against other native components"));
        assert!(text.contains(&set));

        (h.api.render)(h.api.ctx, 0, b"{}".as_ptr(), 2);
        assert!(h.inbox.renders.try_recv().is_err(), "renders are ignored");
        // Platform calls still go through.
        (h.api.call)(h.api.ctx, b"{}".as_ptr(), 2);
        assert!(h.inbox.calls.try_recv().is_ok());
    }

    #[test]
    fn components_missing_from_the_host_are_not_refused() {
        let h = harness();
        let json = br#"{"gpui-nonexistent": "abc"}"#;
        assert!((h.api.check_components)(
            h.api.ctx,
            json.as_ptr(),
            json.len()
        ));
        assert!(h.inbox.renders.try_recv().is_err());
        // Unparseable component sets count as none.
        assert!((h.api.check_components)(h.api.ctx, b"nope".as_ptr(), 4));
    }
}
