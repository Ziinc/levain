//! The C ABI between levain's host and a language runtime.
//!
//! The host and the runtime share one process. The host hands the runtime
//! the address of its [`HostApi`] (a language package passes it to its runtime, for
//! instance on the command line, and the runtime reads it back), so neither side needs the
//! other's symbols. Everything crossing the boundary is plain bytes (JSON).

use std::ffi::c_void;

/// Bumped whenever [`HostApi`] or [`EventSink`] changes shape, or the crate
/// is renamed (3 was an earlier crate name).
pub const ABI_VERSION: u32 = 4;

/// Called by the host, on its UI thread, for every UI event.
pub type EventSinkFn = extern "C" fn(ctx: *mut c_void, json: *const u8, len: usize);

/// What the runtime hands the host so events can reach it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct EventSink {
    pub ctx: *mut c_void,
    pub call: EventSinkFn,
}

/// Functions the host exposes to the runtime. Every function is safe to call
/// from any thread.
#[repr(C)]
pub struct HostApi {
    pub abi_version: u32,
    pub ctx: *mut c_void,
    /// Replaces the view of window `window` with the JSON-encoded tree.
    pub render: extern "C" fn(ctx: *mut c_void, window: u32, json: *const u8, len: usize),
    /// Installs the sink events are delivered to. A later call replaces it.
    pub set_event_sink: extern "C" fn(ctx: *mut c_void, sink: EventSink),
    /// Checks the native component sets the app was compiled against (JSON,
    /// set name to manifest hash) against the host's. On a mismatch the
    /// host shows why, ignores renders and returns `false`.
    pub check_components: extern "C" fn(ctx: *mut c_void, json: *const u8, len: usize) -> bool,
    /// A platform call (JSON `{"call", "ref", "args"}`). The host runs it on
    /// its UI thread and replies with a `reply` event carrying `ref`.
    pub call: extern "C" fn(ctx: *mut c_void, json: *const u8, len: usize),
}

// SAFETY: `HostApi` is only ever shared by address and its functions are
// required to be thread safe.
unsafe impl Send for HostApi {}
unsafe impl Sync for HostApi {}
unsafe impl Send for EventSink {}
unsafe impl Sync for EventSink {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    const P: usize = size_of::<usize>();

    #[test]
    fn version_is_4() {
        assert_eq!(ABI_VERSION, 4);
    }

    // The layout every language's binding reads. It must not change without
    // a new ABI_VERSION.
    #[test]
    fn host_api_layout() {
        assert_eq!(offset_of!(HostApi, abi_version), 0);
        assert_eq!(offset_of!(HostApi, ctx), P);
        assert_eq!(offset_of!(HostApi, render), 2 * P);
        assert_eq!(offset_of!(HostApi, set_event_sink), 3 * P);
        assert_eq!(offset_of!(HostApi, check_components), 4 * P);
        assert_eq!(offset_of!(HostApi, call), 5 * P);
        assert_eq!(size_of::<HostApi>(), 6 * P);
    }

    #[test]
    fn event_sink_layout() {
        assert_eq!(offset_of!(EventSink, ctx), 0);
        assert_eq!(offset_of!(EventSink, call), P);
        assert_eq!(size_of::<EventSink>(), 2 * P);
    }
}
