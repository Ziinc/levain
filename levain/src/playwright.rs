//! Serves the app's windows to Playwright clients (the `playwright` feature).
//! The server only starts when `GPUI_PLAYWRIGHT` is set, such as `9333`;
//! `GPUI_PLAYWRIGHT_ENDPOINT_FILE` names a file it writes its endpoint to.

pub fn start(cx: &mut tauri_plugin_gpui::gpui::App) {
    if let Some(server) = gpui_playwright::Server::from_env() {
        match server.start(cx) {
            Ok(endpoint) => crate::log_line(&format!("baguette: playwright on {endpoint}")),
            Err(error) => crate::log_line(&format!("baguette: playwright failed: {error}")),
        }
    }
}
