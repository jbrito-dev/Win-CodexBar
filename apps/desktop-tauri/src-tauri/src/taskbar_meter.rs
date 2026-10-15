//! Taskbar meter: a small always-on-top window docked onto the Windows
//! taskbar strip.
//!
//! Unlike the free-floating `floatbar`, this window is glued to the taskbar:
//! its cross-axis position is always centered inside the taskbar strip and the
//! user can only slide it along the taskbar (left/right for a bottom or top
//! taskbar). The slide position is stored as a per-mille fraction of the
//! available travel (`Settings::taskbar_meter_position`) so it survives DPI,
//! resolution and monitor changes. While shown, a light guard loop keeps it
//! above the taskbar, re-docks it after display changes, and hides it while a
//! fullscreen app covers the monitor (the taskbar is hidden then too).

use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::time::Duration;

use codexbar::settings::Settings;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl};

pub const TASKBAR_METER_LABEL: &str = "taskbarmeter";

/// Initial window size before the webview reports its content size.
const INITIAL_LOGICAL_SIZE: (f64, f64) = (260.0, 40.0);
/// Taskbar height assumed when the taskbar auto-hides (no reserved strip).
const AUTOHIDE_STRIP_LOGICAL: f64 = 48.0;
/// Guard loop cadence while the meter is shown.
const GUARD_INTERVAL: Duration = Duration::from_millis(400);
/// Largest stored slide position (per mille of the available travel).
pub const POSITION_MAX: u16 = 1000;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static LOOP_RUNNING: AtomicBool = AtomicBool::new(false);
static DRAGGING: AtomicBool = AtomicBool::new(false);
/// Live slide position; persisted to settings when a drag ends.
static POSITION: AtomicU16 = AtomicU16::new(250);
/// Device name of the monitor whose taskbar holds the meter ("" = primary).
static MONITOR: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
/// Hidden by the guard because a fullscreen app covers the monitor.
static HIDDEN_FOR_FULLSCREEN: AtomicBool = AtomicBool::new(false);

/// The taskbar strip of a monitor, in physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Strip {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// True for a bottom/top taskbar (slide horizontally).
    pub horizontal: bool,
}

/// Derive the taskbar strip from a monitor's bounds and work area. With an
/// auto-hidden taskbar there is no reserved strip, so assume a bottom strip of
/// `autohide_height` physical pixels.
pub(crate) fn strip_for(
    monitor: (i32, i32, i32, i32),
    work: (i32, i32, i32, i32),
    autohide_height: i32,
) -> Strip {
    let (mx, my, mw, mh) = monitor;
    let (wx, wy, ww, wh) = work;
    let m_right = mx + mw;
    let m_bottom = my + mh;
    let w_right = wx + ww;
    let w_bottom = wy + wh;
    if w_bottom < m_bottom {
        return Strip {
            x: mx,
            y: w_bottom,
            width: mw,
            height: m_bottom - w_bottom,
            horizontal: true,
        };
    }
    if wy > my {
        return Strip {
            x: mx,
            y: my,
            width: mw,
            height: wy - my,
            horizontal: true,
        };
    }
    if wx > mx {
        return Strip {
            x: mx,
            y: my,
            width: wx - mx,
            height: mh,
            horizontal: false,
        };
    }
    if w_right < m_right {
        return Strip {
            x: w_right,
            y: my,
            width: m_right - w_right,
            height: mh,
            horizontal: false,
        };
    }
    let height = autohide_height.clamp(1, mh.max(1));
    Strip {
        x: mx,
        y: m_bottom - height,
        width: mw,
        height,
        horizontal: true,
    }
}

/// Window origin for a `width`×`height` window at slide `position` (per mille)
/// inside `strip`: centered on the cross axis, slid along the long axis.
pub(crate) fn place(strip: Strip, width: i32, height: i32, position: u16) -> (i32, i32) {
    let p = i64::from(position.min(POSITION_MAX));
    if strip.horizontal {
        let travel = i64::from((strip.width - width).max(0));
        let x = strip.x + i32::try_from(travel * p / i64::from(POSITION_MAX)).unwrap_or(0);
        let y = strip.y + (strip.height - height) / 2;
        (x, y)
    } else {
        let travel = i64::from((strip.height - height).max(0));
        let y = strip.y + i32::try_from(travel * p / i64::from(POSITION_MAX)).unwrap_or(0);
        let x = strip.x + (strip.width - width) / 2;
        (x, y)
    }
}

/// Slide position (per mille) after moving a window at `current` by `delta`
/// physical pixels along the strip, clamped to the strip.
pub(crate) fn slide(strip: Strip, width: i32, height: i32, current: u16, delta: i32) -> u16 {
    let (x, y) = place(strip, width, height, current);
    let (start, along, travel) = if strip.horizontal {
        (strip.x, x, (strip.width - width).max(0))
    } else {
        (strip.y, y, (strip.height - height).max(0))
    };
    if travel == 0 {
        return current.min(POSITION_MAX);
    }
    let offset = (along - start + delta).clamp(0, travel);
    let per_mille = i64::from(offset) * i64::from(POSITION_MAX) / i64::from(travel);
    u16::try_from(per_mille)
        .unwrap_or(POSITION_MAX)
        .min(POSITION_MAX)
}

/// Strip of the primary monitor (where the main taskbar lives).
/// Monitor the user attached the meter to (by device name), else primary.
fn target_monitor(window: &tauri::WebviewWindow) -> Option<tauri::Monitor> {
    let wanted = MONITOR.lock().map(|name| name.clone()).unwrap_or_default();
    if !wanted.is_empty() {
        if let Ok(monitors) = window.available_monitors() {
            if let Some(found) = monitors
                .into_iter()
                .find(|m| m.name().is_some_and(|name| *name == wanted))
            {
                return Some(found);
            }
        }
    }
    window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten())
}

fn primary_strip(window: &tauri::WebviewWindow) -> Option<Strip> {
    let monitor = target_monitor(window)?;
    let pos = monitor.position();
    let size = monitor.size();
    let work = monitor.work_area();
    let to_i32 = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
    let autohide = (AUTOHIDE_STRIP_LOGICAL * monitor.scale_factor()).round();
    Some(strip_for(
        (pos.x, pos.y, to_i32(size.width), to_i32(size.height)),
        (
            work.position.x,
            work.position.y,
            to_i32(work.size.width),
            to_i32(work.size.height),
        ),
        // Bounded by the clamp in `strip_for`; a logical 48px strip fits i32.
        autohide.clamp(1.0, 4096.0) as i32,
    ))
}

fn window_size(window: &tauri::WebviewWindow) -> (i32, i32) {
    window
        .outer_size()
        .map(|s| {
            (
                i32::try_from(s.width).unwrap_or(i32::MAX),
                i32::try_from(s.height).unwrap_or(i32::MAX),
            )
        })
        .unwrap_or((1, 1))
}

/// Dock the window at the live slide position.
fn redock(window: &tauri::WebviewWindow) {
    let Some(strip) = primary_strip(window) else {
        return;
    };
    let (w, h) = window_size(window);
    let (x, y) = place(strip, w, h, POSITION.load(Ordering::Relaxed));
    let current = window.outer_position().ok();
    if current.map(|p| (p.x, p.y)) != Some((x, y)) {
        let _moved = window.set_position(PhysicalPosition::new(x, y));
    }
}

/// Open (or re-show) the taskbar meter.
pub fn show(app: &AppHandle) -> Result<(), String> {
    let settings = Settings::load();
    POSITION.store(
        settings.taskbar_meter_position.min(POSITION_MAX),
        Ordering::Relaxed,
    );
    if let Ok(mut name) = MONITOR.lock() {
        name.clone_from(&settings.taskbar_meter_monitor);
    }
    if let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) {
        redock(&window);
        crate::floatbar::apply_no_activate(&window);
        window.show().map_err(|e| e.to_string())?;
        crate::floatbar::apply_always_on_top(&window);
        set_active(app, true);
        return Ok(());
    }

    let url = WebviewUrl::App("index.html?window=taskbarmeter".into());
    let builder = tauri::WebviewWindowBuilder::new(app, TASKBAR_METER_LABEL, url)
        .title(format!("{} Taskbar Meter", codexbar::brand::APP_NAME))
        .inner_size(INITIAL_LOGICAL_SIZE.0, INITIAL_LOGICAL_SIZE.1)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        // Same WebView2 shared-profile reason as the floatbar: pin Dark.
        .theme(Some(tauri::Theme::Dark));
    #[cfg(windows)]
    let builder = builder.transparent(true);
    let window = builder
        .background_color(tauri::utils::config::Color(0, 0, 0, 0))
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;

    window.on_menu_event(|window, event| {
        handle_menu(window.app_handle(), event.id().as_ref());
    });
    redock(&window);
    crate::floatbar::apply_no_activate(&window);
    crate::floatbar::apply_always_on_top(&window);
    window.show().map_err(|e| e.to_string())?;
    crate::floatbar::apply_always_on_top(&window);
    set_active(app, true);
    Ok(())
}

/// Close the taskbar meter.
pub fn hide(app: &AppHandle) -> Result<(), String> {
    ACTIVE.store(false, Ordering::Release);
    if let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) {
        window.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Bring the window in line with `settings.taskbar_meter_enabled`.
pub fn apply_state(app: &AppHandle, settings: &Settings) {
    let open = app.get_webview_window(TASKBAR_METER_LABEL).is_some();
    if settings.taskbar_meter_enabled && !open {
        if let Err(error) = show(app) {
            tracing::warn!("taskbar meter: show failed: {error}");
        }
    } else if !settings.taskbar_meter_enabled && open {
        if let Err(error) = hide(app) {
            tracing::warn!("taskbar meter: hide failed: {error}");
        }
    }
}

/// Startup: reopen the meter if it was enabled.
pub fn install(app: &AppHandle) {
    let settings = Settings::load();
    apply_state(app, &settings);
}

fn set_active(app: &AppHandle, active: bool) {
    ACTIVE.store(active, Ordering::Release);
    if active && !LOOP_RUNNING.swap(true, Ordering::AcqRel) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while ACTIVE.load(Ordering::Acquire) {
                tokio::time::sleep(GUARD_INTERVAL).await;
                let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) else {
                    break;
                };
                guard_tick(&window);
            }
            LOOP_RUNNING.store(false, Ordering::Release);
        });
    }
}

fn guard_tick(window: &tauri::WebviewWindow) {
    let fullscreen = platform::foreground_is_fullscreen();
    if fullscreen {
        if !HIDDEN_FOR_FULLSCREEN.swap(true, Ordering::AcqRel) {
            let _hidden = window.hide();
        }
        return;
    }
    if HIDDEN_FOR_FULLSCREEN.swap(false, Ordering::AcqRel) {
        crate::floatbar::apply_no_activate(window);
        let _shown = window.show();
    }
    if !DRAGGING.load(Ordering::Acquire) {
        redock(window);
    }
    crate::floatbar::apply_always_on_top(window);
}

/// Webview reports its content size (physical pixels): resize and re-dock.
#[tauri::command]
pub fn taskbar_meter_resize(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) else {
        return Ok(());
    };
    // Content sizes are small positive pixel counts; clamp before converting.
    let width = width.ceil().clamp(1.0, 8192.0) as u32;
    let height = height.ceil().clamp(1.0, 8192.0) as u32;
    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    redock(&window);
    crate::floatbar::apply_no_activate(&window);
    crate::floatbar::apply_always_on_top(&window);
    Ok(())
}

/// Slide the meter along the taskbar by the pointer movement (physical
/// pixels); only the component along the taskbar is used.
#[tauri::command]
pub fn taskbar_meter_drag(app: AppHandle, dx: f64, dy: f64) -> Result<(), String> {
    let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) else {
        return Ok(());
    };
    DRAGGING.store(true, Ordering::Release);
    let Some(strip) = primary_strip(&window) else {
        return Ok(());
    };
    let (w, h) = window_size(&window);
    // Drag deltas are bounded by the screen size.
    let delta = if strip.horizontal { dx } else { dy };
    let delta = delta.round().clamp(-16384.0, 16384.0) as i32;
    let next = slide(strip, w, h, POSITION.load(Ordering::Relaxed), delta);
    POSITION.store(next, Ordering::Relaxed);
    redock(&window);
    Ok(())
}

/// Finish a drag: persist the slide position.
#[tauri::command]
pub fn taskbar_meter_drag_end() -> Result<(), String> {
    DRAGGING.store(false, Ordering::Release);
    let mut settings = Settings::load();
    let position = POSITION.load(Ordering::Relaxed);
    if settings.taskbar_meter_position != position {
        settings.taskbar_meter_position = position;
        settings.save().map_err(|e| e.to_string())?;
    }
    Ok(())
}

const MENU_MONITOR_PREFIX: &str = "tbm_monitor:";
const MENU_HIDE: &str = "tbm_hide";

/// Right-click menu: pick the monitor whose taskbar holds the meter, or hide it.
#[tauri::command]
pub fn taskbar_meter_context_menu(app: AppHandle) -> Result<(), String> {
    use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
    let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) else {
        return Ok(());
    };
    let monitors = window.available_monitors().map_err(|e| e.to_string())?;
    let primary_name = window
        .primary_monitor()
        .ok()
        .flatten()
        .and_then(|m| m.name().cloned());
    let current = target_monitor(&window).and_then(|m| m.name().cloned());
    let menu = Menu::new(&app).map_err(|e| e.to_string())?;
    let header = MenuItem::with_id(
        &app,
        "tbm_header",
        "Show on taskbar of:",
        false,
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    menu.append(&header).map_err(|e| e.to_string())?;
    for (index, monitor) in monitors.iter().enumerate() {
        let Some(name) = monitor.name() else { continue };
        let size = monitor.size();
        let mut label = format!("Display {} — {}×{}", index + 1, size.width, size.height);
        if primary_name.as_ref() == Some(name) {
            label.push_str(" (primary)");
        }
        let item = CheckMenuItem::with_id(
            &app,
            format!("{MENU_MONITOR_PREFIX}{name}"),
            label,
            true,
            current.as_ref() == Some(name),
            None::<&str>,
        )
        .map_err(|e| e.to_string())?;
        menu.append(&item).map_err(|e| e.to_string())?;
    }
    let separator = PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?;
    menu.append(&separator).map_err(|e| e.to_string())?;
    let hide = MenuItem::with_id(&app, MENU_HIDE, "Hide taskbar meter", true, None::<&str>)
        .map_err(|e| e.to_string())?;
    menu.append(&hide).map_err(|e| e.to_string())?;
    window.popup_menu(&menu).map_err(|e| e.to_string())
}

fn handle_menu(app: &AppHandle, id: &str) {
    if let Some(name) = id.strip_prefix(MENU_MONITOR_PREFIX) {
        if let Ok(mut current) = MONITOR.lock() {
            *current = name.to_string();
        }
        let mut settings = Settings::load();
        settings.taskbar_meter_monitor = name.to_string();
        if let Err(error) = settings.save() {
            tracing::warn!("taskbar meter: saving monitor failed: {error}");
        }
        if let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) {
            redock(&window);
            crate::floatbar::apply_always_on_top(&window);
        }
    } else if id == MENU_HIDE {
        let mut settings = Settings::load();
        settings.taskbar_meter_enabled = false;
        if let Err(error) = settings.save() {
            tracing::warn!("taskbar meter: saving hide failed: {error}");
        }
        if let Err(error) = hide(app) {
            tracing::warn!("taskbar meter: hide failed: {error}");
        }
        crate::events::emit_settings_changed(app);
    }
}

#[cfg(windows)]
mod platform {
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct MonitorInfo {
        cb_size: u32,
        rc_monitor: Rect,
        rc_work: Rect,
        dw_flags: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> isize;
        fn GetDesktopWindow() -> isize;
        fn GetShellWindow() -> isize;
        fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
        fn GetClassNameW(hwnd: isize, name: *mut u16, max: i32) -> i32;
        fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
        fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
    }

    const MONITOR_DEFAULTTONEAREST: u32 = 2;

    /// True when the foreground window covers its whole monitor (a game,
    /// video or presentation), excluding the desktop and the taskbar itself.
    pub fn foreground_is_fullscreen() -> bool {
        // SAFETY: plain Win32 queries on window handles; every out-pointer is
        // a valid, properly sized local.
        unsafe {
            let fg = GetForegroundWindow();
            if fg == 0 || fg == GetDesktopWindow() || fg == GetShellWindow() {
                return false;
            }
            let mut class = [0u16; 64];
            let len = GetClassNameW(fg, class.as_mut_ptr(), 64);
            if len > 0 {
                let name = String::from_utf16_lossy(&class[..usize::try_from(len).unwrap_or(0)]);
                if matches!(
                    name.as_str(),
                    "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
                ) {
                    return false;
                }
            }
            let mut rect = Rect::default();
            if GetWindowRect(fg, &mut rect) == 0 {
                return false;
            }
            let monitor = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
            if monitor == 0 {
                return false;
            }
            let mut info = MonitorInfo {
                cb_size: u32::try_from(std::mem::size_of::<MonitorInfo>()).unwrap_or(40),
                rc_monitor: Rect::default(),
                rc_work: Rect::default(),
                dw_flags: 0,
            };
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return false;
            }
            let m = info.rc_monitor;
            rect.left <= m.left
                && rect.top <= m.top
                && rect.right >= m.right
                && rect.bottom >= m.bottom
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn foreground_is_fullscreen() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MON: (i32, i32, i32, i32) = (0, 0, 1920, 1080);

    #[test]
    fn bottom_taskbar_strip() {
        let strip = strip_for(MON, (0, 0, 1920, 1032), 48);
        assert_eq!(
            strip,
            Strip {
                x: 0,
                y: 1032,
                width: 1920,
                height: 48,
                horizontal: true
            }
        );
    }

    #[test]
    fn top_and_side_taskbars() {
        assert_eq!(strip_for(MON, (0, 40, 1920, 1040), 48).y, 0);
        let left = strip_for(MON, (60, 0, 1860, 1080), 48);
        assert!(!left.horizontal);
        assert_eq!(left.width, 60);
        let right = strip_for(MON, (0, 0, 1860, 1080), 48);
        assert_eq!(right.x, 1860);
    }

    #[test]
    fn autohide_assumes_bottom_strip() {
        let strip = strip_for(MON, MON, 48);
        assert_eq!(
            strip,
            Strip {
                x: 0,
                y: 1032,
                width: 1920,
                height: 48,
                horizontal: true
            }
        );
    }

    #[test]
    fn place_centers_vertically_and_slides_horizontally() {
        let strip = strip_for(MON, (0, 0, 1920, 1032), 48);
        assert_eq!(place(strip, 320, 40, 0), (0, 1036));
        assert_eq!(place(strip, 320, 40, 1000), (1600, 1036));
        assert_eq!(place(strip, 320, 40, 500), (800, 1036));
    }

    #[test]
    fn slide_clamps_to_the_strip() {
        let strip = strip_for(MON, (0, 0, 1920, 1032), 48);
        assert_eq!(slide(strip, 320, 40, 500, 160), 600);
        assert_eq!(slide(strip, 320, 40, 500, -5000), 0);
        assert_eq!(slide(strip, 320, 40, 500, 5000), 1000);
    }
}
