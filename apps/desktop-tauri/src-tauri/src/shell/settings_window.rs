//! Detached Settings window: opens Settings/About in a separate window
//! so the tray panel stays open.

use tauri::{Emitter, Manager, PhysicalPosition, WebviewUrl};

use super::activation::{self, Activation};

const SETTINGS_LABEL: &str = "settings";
use crate::surface::{
    SETTINGS_WINDOW_HEIGHT, SETTINGS_WINDOW_MIN_HEIGHT, SETTINGS_WINDOW_MIN_WIDTH,
    SETTINGS_WINDOW_WIDTH,
};

/// Open the detached Settings window, or focus it if already open.
///
/// Every caller is a user action (the tray menu or a Settings button in
/// CodexBar's own UI), so the window takes focus; proof mode still keeps it
/// unfocused (see [`super::activation`]).
///
/// When the window already exists, emits `settings-change-tab` so the
/// frontend can switch to the requested tab without a full reload.
pub fn open_or_focus(app: &tauri::AppHandle, tab: &str) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(SETTINGS_LABEL) {
        window.show().map_err(|e| e.to_string())?;
        activation::apply(&window, Activation::UserAction)?;
        app.emit_to(SETTINGS_LABEL, "settings-change-tab", tab)
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let url = WebviewUrl::App(format!("index.html?window=settings&tab={tab}").into());

    // `focused(false)`: showing the window never activates it by itself;
    // `activation::apply` below decides whether it takes focus.
    let win = tauri::WebviewWindowBuilder::new(app, SETTINGS_LABEL, url)
        .title(format!("{} Settings", codexbar::brand::APP_NAME))
        .inner_size(SETTINGS_WINDOW_WIDTH, SETTINGS_WINDOW_HEIGHT)
        .min_inner_size(SETTINGS_WINDOW_MIN_WIDTH, SETTINGS_WINDOW_MIN_HEIGHT)
        .decorations(false)
        .shadow(false)
        .theme(Some(tauri::Theme::Dark))
        .resizable(true)
        .focused(false)
        .build()
        .map_err(|e| e.to_string())?;

    // Force DWM caption to dark; keep WS_THICKFRAME since window is resizable
    super::dwm::force_dark_caption_resizable(&win);

    // Manually center: Tauri's .center() is unreliable on Windows when
    // called from async commands. Compute position from the primary monitor.
    if let Ok(Some(monitor)) = win.primary_monitor() {
        let pos = monitor.position();
        let size = monitor.size();
        let scale = win.scale_factor().unwrap_or(1.0);
        let win_w = (SETTINGS_WINDOW_WIDTH * scale) as i32;
        let win_h = (SETTINGS_WINDOW_HEIGHT * scale) as i32;
        let x = pos.x + (size.width as i32 - win_w) / 2;
        let y = pos.y + (size.height as i32 - win_h) / 2;
        let _ = win.set_position(PhysicalPosition::new(x, y));
    }

    activation::apply(&win, Activation::UserAction)
}

/// Dismiss Settings without exiting CodexBar.
///
/// The detached Settings window is hidden instead of closed so Tauri's
/// process/window lifecycle cannot interpret this as an app quit. If Settings
/// is rendered in the main shell surface, hide that surface back to tray.
pub fn dismiss(app: &tauri::AppHandle, window: &tauri::WebviewWindow) -> Result<(), String> {
    if window.label() == SETTINGS_LABEL {
        return window.hide().map_err(|e| e.to_string());
    }

    crate::shell::hide_to_tray_if_current(app, |mode| {
        mode == crate::surface::SurfaceMode::Settings
    })?;
    Ok(())
}
