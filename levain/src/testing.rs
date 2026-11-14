//! Headless windows for tests: a levain `Root` in GPUI's test platform, as
//! `windows::open` puts one in a Tauri window.

use gpui_kit::{
    AnyWindowHandle, Bounds, Entity, Point, TestAppContext, VisualTestContext, WindowBounds,
    WindowOptions, px, size,
};

use crate::view::{Frame, Message, Root};

/// The frame in `json`.
pub fn frame(json: &str) -> Frame {
    match serde_json::from_str(json).expect("a frame") {
        Message::Frame(frame) => frame,
        _ => panic!("not a frame"),
    }
}

/// Opens window `id`, 400x300 and empty, as `windows::open` mounts one.
pub fn window(cx: &mut TestAppContext, id: u32) -> AnyWindowHandle {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(400.), px(300.)),
                })),
                ..Default::default()
            },
            |window, cx| crate::windows::mount(id, Frame::default(), window, cx),
        )
        .expect("a window")
        .into()
    })
}

/// Window 0 showing `json`, drawn once.
pub fn open<'a>(
    cx: &'a mut TestAppContext,
    json: &str,
) -> (Entity<Root>, &'a mut VisualTestContext) {
    let handle = window(cx, 0);
    let view = cx.update(|cx| crate::windows::root_of(0, cx).expect("window 0's view"));
    let cx = VisualTestContext::from_window(handle, cx).into_mut();
    show(&view, cx, json);
    (view, cx)
}

/// Shows the frame in `json` and draws it.
pub fn show(view: &Entity<Root>, cx: &mut VisualTestContext, json: &str) {
    let frame = frame(json);
    view.update(cx, |root, cx| root.show(frame, cx));
    draw(cx);
}

/// Draws the window again, as its next frame would.
pub fn draw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.refresh();
        window.draw(cx).clear(cx);
    });
}
