//! Taskbar meter: a small always-on-top window docked onto the Windows
//! taskbar strip.
//!
//! Unlike the free-floating `floatbar`, this window is glued to the taskbar:
//! its cross-axis position is always centered inside the taskbar strip and the
//! user can only slide it along the taskbar (left/right for a bottom or top
//! taskbar). The slide position is stored as a per-mille fraction of the
//! available travel (`Settings::taskbar_meter_position`) so it survives DPI,
//! resolution and monitor changes.
//!
//! The meter is only visible while its monitor's taskbar is: it hides while a
//! window spanning the monitor sits above the taskbar in z-order (a game,
//! video or presentation) and re-raises itself only when Explorer actually
//! put the taskbar over it. Shell surfaces such as the Start menu, search and
//! flyouts don't count as fullscreen. WinEvent hooks react to foreground and
//! z-order changes immediately; a light guard loop is the fallback and also
//! re-docks the meter after display changes.

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
const GUARD_INTERVAL: Duration = Duration::from_millis(150);
/// Largest stored slide position (per mille of the available travel).
pub const POSITION_MAX: u16 = 1000;

static ACTIVE: AtomicBool = AtomicBool::new(false);
static LOOP_RUNNING: AtomicBool = AtomicBool::new(false);
static DRAGGING: AtomicBool = AtomicBool::new(false);
/// Position locked: presses on the meter don't start a drag.
static LOCKED: AtomicBool = AtomicBool::new(false);
/// Cursor-poll cadence while a drag is in progress.
const DRAG_POLL: Duration = Duration::from_millis(8);
/// Cursor travel (physical px) before a press counts as a drag.
const DRAG_THRESHOLD_PX: i32 = 3;
/// Live slide position; persisted to settings when a drag ends.
static POSITION: AtomicU16 = AtomicU16::new(250);
/// Device name of the monitor whose taskbar holds the meter ("" = primary).
static MONITOR: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());
/// Hidden by the guard because a fullscreen window covers the taskbar.
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
    if !wanted.is_empty()
        && let Ok(monitors) = window.available_monitors()
        && let Some(found) = monitors
            .into_iter()
            .find(|m| m.name().is_some_and(|name| *name == wanted))
    {
        return Some(found);
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

/// Center of the target monitor (physical px), used to find its taskbar.
fn target_point(window: &tauri::WebviewWindow) -> Option<(i32, i32)> {
    let monitor = target_monitor(window)?;
    let pos = monitor.position();
    let size = monitor.size();
    let half = |v: u32| i32::try_from(v / 2).unwrap_or(0);
    Some((pos.x + half(size.width), pos.y + half(size.height)))
}

/// Strip the meter slides along: the taskbar window it is embedded in, else
/// the strip derived from the monitor work area.
fn current_strip(window: &tauri::WebviewWindow) -> Option<Strip> {
    if let Some(host) = platform::host_rect(window) {
        return Some(host);
    }
    primary_strip(window)
}

/// Dock the window at the live slide position. Embeds it into the target
/// monitor's taskbar first (so it is drawn as part of the taskbar), falling
/// back to a free-floating topmost window when no taskbar window is found.
fn redock(window: &tauri::WebviewWindow) {
    let embedded = target_point(window).and_then(|point| platform::embed(window, point));
    let Some(strip) = embedded.or_else(|| primary_strip(window)) else {
        return;
    };
    let (w, h) = window_size(window);
    let (x, y) = place(strip, w, h, POSITION.load(Ordering::Relaxed));
    let current = window.outer_position().ok();
    if current.map(|p| (p.x, p.y)) == Some((x, y)) {
        return;
    }
    if embedded.is_some() {
        // Child windows are positioned in the taskbar's client coordinates.
        platform::move_child(window, x - strip.x, y - strip.y);
    } else {
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
    LOCKED.store(settings.taskbar_meter_locked, Ordering::Release);
    if let Ok(mut name) = MONITOR.lock() {
        name.clone_from(&settings.taskbar_meter_monitor);
    }
    if let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) {
        redock(&window);
        crate::floatbar::apply_no_activate(&window);
        window.show().map_err(|e| e.to_string())?;
        raise(&window);
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
    raise(&window);
    window.show().map_err(|e| e.to_string())?;
    raise(&window);
    set_active(app, true);
    Ok(())
}

/// Keep a free-floating (not embedded) meter above the taskbar. An embedded
/// meter is part of the taskbar and needs nothing.
fn raise(window: &tauri::WebviewWindow) {
    if !platform::is_embedded() {
        crate::floatbar::apply_always_on_top(window);
    }
}

/// Lock or unlock the meter's position (settings toggle / context menu).
pub fn set_locked(locked: bool) {
    LOCKED.store(locked, Ordering::Release);
}

/// Close the taskbar meter.
pub fn hide(app: &AppHandle) -> Result<(), String> {
    ACTIVE.store(false, Ordering::Release);
    platform::forget_window();
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
    } else if !settings.taskbar_meter_enabled
        && open
        && let Err(error) = hide(app)
    {
        tracing::warn!("taskbar meter: hide failed: {error}");
    }
}

/// Startup: reopen the meter if it was enabled.
pub fn install(app: &AppHandle) {
    let settings = Settings::load();
    apply_state(app, &settings);
}

fn set_active(app: &AppHandle, active: bool) {
    ACTIVE.store(active, Ordering::Release);
    if active {
        // React instantly whenever Explorer reorders windows or a fullscreen
        // app takes the foreground; the poll below is only the fallback.
        if let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) {
            platform::track_window(&window);
        }
        let _hooked = app.run_on_main_thread(platform::install_zorder_hook);
        let _ = APP.set(app.clone());
        platform::install_mouse_hook();
    } else {
        platform::forget_window();
    }
    if active && !LOOP_RUNNING.swap(true, Ordering::AcqRel) {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while ACTIVE.load(Ordering::Acquire) {
                tokio::time::sleep(GUARD_INTERVAL).await;
                let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) else {
                    // Destroyed with its host taskbar (Explorer restart):
                    // recreate it once a taskbar is back.
                    if ACTIVE.load(Ordering::Acquire) && platform::taskbar_exists() {
                        platform::forget_window();
                        if let Err(error) = show(&app) {
                            tracing::warn!("taskbar meter: recreate failed: {error}");
                        }
                    }
                    continue;
                };
                guard_tick(&window);
            }
            LOOP_RUNNING.store(false, Ordering::Release);
        });
    }
}

fn guard_tick(window: &tauri::WebviewWindow) {
    if !DRAGGING.load(Ordering::Acquire) {
        redock(window);
    }
    if platform::is_embedded() {
        // Part of the taskbar: shown, hidden and layered with it.
        if HIDDEN_FOR_FULLSCREEN.swap(false, Ordering::AcqRel) {
            platform::set_shown(window, true);
        }
        return;
    }
    if platform::taskbar_is_covered(window) {
        if !HIDDEN_FOR_FULLSCREEN.swap(true, Ordering::AcqRel) {
            platform::set_shown(window, false);
        }
        return;
    }
    if HIDDEN_FOR_FULLSCREEN.swap(false, Ordering::AcqRel) {
        crate::floatbar::apply_no_activate(window);
        platform::set_shown(window, true);
    }
    raise(window);
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
    raise(&window);
    Ok(())
}

/// A primary-button press on the meter: track the cursor natively until the
/// physical button is released, sliding the meter along the taskbar. Polling
/// the button state (instead of waiting for a webview pointerup, which can be
/// lost while the window moves under the cursor) guarantees the drag ends on
/// release. Ignored while the position is locked.
#[tauri::command]
pub fn taskbar_meter_drag_start(app: AppHandle) -> Result<(), String> {
    begin_drag(&app);
    Ok(())
}

fn begin_drag(app: &AppHandle) {
    if LOCKED.load(Ordering::Acquire) {
        return;
    }
    let Some(window) = app.get_webview_window(TASKBAR_METER_LABEL) else {
        return;
    };
    let Some(origin) = platform::cursor_position() else {
        return;
    };
    if DRAGGING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(move || {
        let moved = drag_loop(&window, origin);
        DRAGGING.store(false, Ordering::Release);
        if moved {
            persist_position();
        }
    });
}

/// App handle for the native mouse hook (set while the meter is active).
static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

/// Mouse hook: primary press on the embedded meter.
fn hook_press() {
    if let Some(app) = APP.get() {
        begin_drag(app);
    }
}

/// Mouse hook: secondary click on the embedded meter. Deferred to the UI
/// thread so the hook callback returns immediately.
fn hook_context_menu() {
    if let Some(app) = APP.get() {
        let handle = app.clone();
        let _queued = app.run_on_main_thread(move || {
            if let Err(error) = taskbar_meter_context_menu(handle) {
                tracing::warn!("taskbar meter: context menu failed: {error}");
            }
        });
    }
}

/// Follow the cursor while the primary button is held. Returns whether the
/// meter moved.
fn drag_loop(window: &tauri::WebviewWindow, origin: (i32, i32)) -> bool {
    let start = POSITION.load(Ordering::Relaxed);
    let mut moved = false;
    while platform::primary_button_down() && !LOCKED.load(Ordering::Acquire) {
        std::thread::sleep(DRAG_POLL);
        let (Some(cursor), Some(strip)) = (platform::cursor_position(), current_strip(window))
        else {
            break;
        };
        let delta = if strip.horizontal {
            cursor.0 - origin.0
        } else {
            cursor.1 - origin.1
        };
        if !moved && delta.abs() < DRAG_THRESHOLD_PX {
            continue;
        }
        moved = true;
        let (w, h) = window_size(window);
        let next = slide(strip, w, h, start, delta);
        if POSITION.swap(next, Ordering::Relaxed) != next {
            redock(window);
        }
    }
    moved
}

/// Persist the live slide position after a drag.
fn persist_position() {
    let mut settings = Settings::load();
    let position = POSITION.load(Ordering::Relaxed);
    if settings.taskbar_meter_position != position {
        settings.taskbar_meter_position = position;
        if let Err(error) = settings.save() {
            tracing::warn!("taskbar meter: saving position failed: {error}");
        }
    }
}

const MENU_MONITOR_PREFIX: &str = "tbm_monitor:";
const MENU_HIDE: &str = "tbm_hide";
const MENU_LOCK: &str = "tbm_lock";

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
    let lock = CheckMenuItem::with_id(
        &app,
        MENU_LOCK,
        "Lock position",
        true,
        LOCKED.load(Ordering::Acquire),
        None::<&str>,
    )
    .map_err(|e| e.to_string())?;
    menu.append(&lock).map_err(|e| e.to_string())?;
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
            raise(&window);
        }
    } else if id == MENU_LOCK {
        let locked = !LOCKED.load(Ordering::Acquire);
        set_locked(locked);
        let mut settings = Settings::load();
        settings.taskbar_meter_locked = locked;
        if let Err(error) = settings.save() {
            tracing::warn!("taskbar meter: saving lock failed: {error}");
        }
        crate::events::emit_settings_changed(app);
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
    use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

    /// HWND of the meter while it is shown (0 when not).
    static METER_HWND: AtomicIsize = AtomicIsize::new(0);
    static HOOKED: AtomicBool = AtomicBool::new(false);
    /// The meter is a child window of a taskbar (see `embed`).
    static EMBEDDED: AtomicBool = AtomicBool::new(false);

    const GWL_STYLE: i32 = -16;
    const WS_POPUP: isize = 0x8000_0000_u32 as i32 as isize;
    const WS_CHILD: isize = 0x4000_0000;
    const GW_CHILD: u32 = 5;
    const HWND_TOP: isize = 0;
    const SWP_NOZORDER: u32 = 0x0004;

    const EVENT_SYSTEM_FOREGROUND: u32 = 0x0003;
    const EVENT_OBJECT_REORDER: u32 = 0x8004;
    const WINEVENT_OUTOFCONTEXT: u32 = 0x0000;
    const WINEVENT_SKIPOWNPROCESS: u32 = 0x0002;
    const HWND_TOPMOST: isize = -1;
    const SWP_NOSIZE: u32 = 0x0001;
    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const GW_HWNDNEXT: u32 = 2;
    const GW_HWNDPREV: u32 = 3;
    const GWL_EXSTYLE: i32 = -20;
    const SW_HIDE: i32 = 0;
    const SW_SHOWNOACTIVATE: i32 = 4;
    const WS_EX_TRANSPARENT: isize = 0x0000_0020;
    const MONITOR_DEFAULTTONEAREST: u32 = 2;
    const DWMWA_CLOAKED: u32 = 14;
    const SM_SWAPBUTTON: i32 = 23;
    const VK_LBUTTON: i32 = 0x01;
    const VK_RBUTTON: i32 = 0x02;
    const QUNS_RUNNING_D3D_FULL_SCREEN: i32 = 3;
    /// Upper bound on the z-order walk above the taskbar.
    const MAX_WALK: usize = 512;

    /// Shell surfaces that sit above the taskbar or span the monitor without
    /// hiding it (Start, search, notification center, Alt+Tab, task view...).
    const SHELL_CLASSES: &[&str] = &[
        "Progman",
        "WorkerW",
        "Shell_TrayWnd",
        "Shell_SecondaryTrayWnd",
        "Windows.UI.Core.CoreWindow",
        "XamlExplorerHostIslandWindow",
        "TopLevelWindowForOverflowXamlIsland",
        "Xaml_WindowedPopupClass",
        "NotifyIconOverflowWindow",
        "MultitaskingViewFrame",
        "ForegroundStaging",
        "TaskListThumbnailWnd",
    ];

    type WinEventProc = unsafe extern "system" fn(isize, u32, isize, i32, i32, u32, u32);

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct Point {
        x: i32,
        y: i32,
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
        fn SetWinEventHook(
            event_min: u32,
            event_max: u32,
            module: isize,
            callback: WinEventProc,
            process: u32,
            thread: u32,
            flags: u32,
        ) -> isize;
        fn SetWindowPos(
            hwnd: isize,
            after: isize,
            x: i32,
            y: i32,
            cx: i32,
            cy: i32,
            flags: u32,
        ) -> i32;
        fn FindWindowExW(
            parent: isize,
            after: isize,
            class: *const u16,
            title: *const u16,
        ) -> isize;
        fn GetWindow(hwnd: isize, cmd: u32) -> isize;
        fn GetTopWindow(hwnd: isize) -> isize;
        fn GetWindowLongPtrW(hwnd: isize, index: i32) -> isize;
        fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
        fn SetWindowLongPtrW(hwnd: isize, index: i32, value: isize) -> isize;
        fn SetParent(child: isize, parent: isize) -> isize;
        fn GetParent(hwnd: isize) -> isize;
        fn MonitorFromPoint(point: Point, flags: u32) -> isize;
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn GetForegroundWindow() -> isize;
        fn GetWindowRect(hwnd: isize, rect: *mut Rect) -> i32;
        fn GetClassNameW(hwnd: isize, name: *mut u16, max: i32) -> i32;
        fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
        fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
        fn GetCursorPos(point: *mut Point) -> i32;
        fn GetAsyncKeyState(key: i32) -> i16;
        fn GetSystemMetrics(index: i32) -> i32;
    }

    #[link(name = "dwmapi")]
    unsafe extern "system" {
        fn DwmGetWindowAttribute(hwnd: isize, attr: u32, value: *mut u32, size: u32) -> i32;
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHQueryUserNotificationState(state: *mut i32) -> i32;
    }

    fn meter_hwnd(window: &tauri::WebviewWindow) -> Option<isize> {
        use raw_window_handle::HasWindowHandle;
        let handle = window.window_handle().ok()?;
        match handle.as_raw() {
            raw_window_handle::RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
            _ => None,
        }
    }

    pub fn track_window(window: &tauri::WebviewWindow) {
        if let Some(hwnd) = meter_hwnd(window) {
            METER_HWND.store(hwnd, Ordering::Release);
        }
    }

    pub fn forget_window() {
        METER_HWND.store(0, Ordering::Release);
        EMBEDDED.store(false, Ordering::Release);
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn class_name(hwnd: isize) -> String {
        let mut class = [0u16; 128];
        // SAFETY: the buffer is a valid local of the advertised length.
        let len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), 128) };
        String::from_utf16_lossy(&class[..usize::try_from(len).unwrap_or(0)])
    }

    fn monitor_rect(monitor: isize) -> Option<Rect> {
        let mut info = MonitorInfo {
            cb_size: u32::try_from(std::mem::size_of::<MonitorInfo>()).unwrap_or(40),
            rc_monitor: Rect::default(),
            rc_work: Rect::default(),
            dw_flags: 0,
        };
        // SAFETY: `info` is a valid, correctly sized MONITORINFO.
        (unsafe { GetMonitorInfoW(monitor, &mut info) } != 0).then_some(info.rc_monitor)
    }

    /// The taskbar window (primary or secondary) on `monitor`, or 0.
    fn taskbar_on(monitor: isize) -> isize {
        let primary = wide("Shell_TrayWnd");
        let secondary = wide("Shell_SecondaryTrayWnd");
        // SAFETY: class names are NUL-terminated locals; window queries only.
        unsafe {
            let main = FindWindowExW(0, 0, primary.as_ptr(), std::ptr::null());
            if main != 0 && MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST) == monitor {
                return main;
            }
            let mut after = 0;
            for _ in 0..16 {
                after = FindWindowExW(0, after, secondary.as_ptr(), std::ptr::null());
                if after == 0 {
                    break;
                }
                if MonitorFromWindow(after, MONITOR_DEFAULTTONEAREST) == monitor {
                    return after;
                }
            }
        }
        0
    }

    /// Whether `taskbar` sits above `meter` in z-order.
    fn below(meter: isize, taskbar: isize) -> bool {
        // SAFETY: z-order walk over top-level windows; read-only queries.
        unsafe {
            let mut hwnd = GetWindow(meter, GW_HWNDPREV);
            for _ in 0..MAX_WALK {
                if hwnd == 0 {
                    return false;
                }
                if hwnd == taskbar {
                    return true;
                }
                hwnd = GetWindow(hwnd, GW_HWNDPREV);
            }
        }
        false
    }

    fn set_topmost(meter: isize) {
        // SAFETY: raising our own window in the topmost band; no pointers.
        unsafe {
            SetWindowPos(
                meter,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE,
            );
        }
    }

    /// Put the meter back above its monitor's taskbar, touching z-order only
    /// when Explorer actually raised the taskbar over it (each needless
    /// re-raise is itself a repaint). Without a taskbar window, force topmost.
    fn raise_hwnd(meter: isize) {
        // SAFETY: window queries only.
        let monitor = unsafe { MonitorFromWindow(meter, MONITOR_DEFAULTTONEAREST) };
        let taskbar = taskbar_on(monitor);
        if taskbar == 0 || below(meter, taskbar) {
            set_topmost(meter);
        }
    }

    pub fn is_embedded() -> bool {
        EMBEDDED.load(Ordering::Acquire)
    }

    pub fn taskbar_exists() -> bool {
        let primary = wide("Shell_TrayWnd");
        // SAFETY: NUL-terminated class name; lookup only.
        unsafe { FindWindowExW(0, 0, primary.as_ptr(), std::ptr::null()) != 0 }
    }

    fn strip_of(hwnd: isize) -> Option<super::Strip> {
        let mut rect = Rect::default();
        // SAFETY: `rect` is a valid out-pointer.
        if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
            return None;
        }
        let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
        (width > 0 && height > 0).then_some(super::Strip {
            x: rect.left,
            y: rect.top,
            width,
            height,
            horizontal: width >= height,
        })
    }

    /// Embed the meter as a child window of the taskbar on the monitor at
    /// `point`, so Windows draws it as part of the taskbar: it stays visible
    /// while Start / search / flyouts lift the taskbar into a higher z-band
    /// (no ordinary topmost window can follow it there), never flashes on
    /// taskbar clicks, and disappears with the taskbar under fullscreen apps.
    /// Returns the taskbar's screen rect, or None (not embedded) when that
    /// monitor has no taskbar window.
    pub fn embed(window: &tauri::WebviewWindow, point: (i32, i32)) -> Option<super::Strip> {
        let meter = meter_hwnd(window)?;
        let point = Point {
            x: point.0,
            y: point.1,
        };
        // SAFETY: window queries/updates on our own window and the taskbar.
        unsafe {
            let taskbar = taskbar_on(MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST));
            if taskbar == 0 {
                if GetParent(meter) != 0 {
                    SetParent(meter, 0);
                    let style = GetWindowLongPtrW(meter, GWL_STYLE);
                    SetWindowLongPtrW(meter, GWL_STYLE, (style & !WS_CHILD) | WS_POPUP);
                }
                EMBEDDED.store(false, Ordering::Release);
                return None;
            }
            if GetParent(meter) != taskbar {
                let style = GetWindowLongPtrW(meter, GWL_STYLE);
                SetWindowLongPtrW(meter, GWL_STYLE, (style & !WS_POPUP) | WS_CHILD);
                SetParent(meter, taskbar);
                if GetParent(meter) != taskbar {
                    SetWindowLongPtrW(meter, GWL_STYLE, style);
                    EMBEDDED.store(false, Ordering::Release);
                    return None;
                }
            }
            // Above the taskbar's own content (its XAML island child).
            if GetWindow(taskbar, GW_CHILD) != meter {
                SetWindowPos(
                    meter,
                    HWND_TOP,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE,
                );
            }
            EMBEDDED.store(true, Ordering::Release);
            strip_of(taskbar)
        }
    }

    /// Screen rect of the taskbar the meter is embedded in.
    pub fn host_rect(window: &tauri::WebviewWindow) -> Option<super::Strip> {
        if !is_embedded() {
            return None;
        }
        // SAFETY: window query only.
        let parent = unsafe { GetParent(meter_hwnd(window)?) };
        if parent == 0 {
            return None;
        }
        strip_of(parent)
    }

    /// Move the embedded meter to (x, y) in its taskbar's client coordinates.
    pub fn move_child(window: &tauri::WebviewWindow, x: i32, y: i32) {
        if let Some(meter) = meter_hwnd(window) {
            // SAFETY: moving our own window.
            unsafe {
                SetWindowPos(
                    meter,
                    0,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
            }
        }
    }

    /// Show/hide without activating. Raw calls on purpose: the z-order hook
    /// hides with ShowWindow too, which tao does not track, so its own
    /// show() would be a no-op afterwards.
    pub fn set_shown(window: &tauri::WebviewWindow, shown: bool) {
        if let Some(meter) = meter_hwnd(window) {
            // SAFETY: showing/hiding our own window.
            unsafe {
                ShowWindow(meter, if shown { SW_SHOWNOACTIVATE } else { SW_HIDE });
            }
        }
    }

    /// React to foreground / z-order changes right away instead of waiting
    /// for the guard poll: hide under a fullscreen window, otherwise make sure
    /// the taskbar didn't just cover the meter.
    unsafe extern "system" fn on_zorder_event(
        _hook: isize,
        _event: u32,
        _hwnd: isize,
        _object: i32,
        _child: i32,
        _thread: u32,
        _time: u32,
    ) {
        let meter = METER_HWND.load(Ordering::Acquire);
        if meter == 0 || EMBEDDED.load(Ordering::Acquire) {
            return;
        }
        if covered(meter) {
            // SAFETY: hiding our own window; the guard shows it again.
            unsafe {
                if IsWindowVisible(meter) != 0 {
                    ShowWindow(meter, SW_HIDE);
                }
            }
            super::HIDDEN_FOR_FULLSCREEN.store(true, Ordering::Release);
            return;
        }
        raise_hwnd(meter);
    }

    /// Install (once, on the UI thread, which pumps messages) out-of-context
    /// hooks for foreground changes and z-order changes in other processes.
    pub fn install_zorder_hook() {
        if HOOKED.swap(true, Ordering::AcqRel) {
            return;
        }
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        // SAFETY: the callback is a plain function with the WINEVENTPROC
        // signature; hooks live for the rest of the process.
        unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                0,
                on_zorder_event,
                0,
                0,
                flags,
            );
            SetWinEventHook(
                EVENT_OBJECT_REORDER,
                EVENT_OBJECT_REORDER,
                0,
                on_zorder_event,
                0,
                0,
                flags,
            );
        }
    }

    /// A visible, opaque, non-shell window that spans the whole monitor.
    fn covers_monitor(hwnd: isize, monitor: Rect) -> bool {
        // SAFETY: window queries with valid, properly sized out-pointers.
        unsafe {
            if IsWindowVisible(hwnd) == 0 {
                return false;
            }
            if GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TRANSPARENT != 0 {
                // Click-through overlays (GPU/recording HUDs) hide nothing.
                return false;
            }
            let mut cloaked = 0u32;
            if DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut cloaked, 4) == 0 && cloaked != 0 {
                return false;
            }
            let mut rect = Rect::default();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return false;
            }
            if !(rect.left <= monitor.left
                && rect.top <= monitor.top
                && rect.right >= monitor.right
                && rect.bottom >= monitor.bottom)
            {
                return false;
            }
        }
        !SHELL_CLASSES.contains(&class_name(hwnd).as_str())
    }

    /// True while the taskbar of the meter's monitor is hidden behind a
    /// fullscreen window (a game, video or presentation): some window that
    /// spans the monitor sits above the taskbar in z-order. Shell surfaces
    /// like the Start menu don't count.
    pub fn taskbar_is_covered(window: &tauri::WebviewWindow) -> bool {
        meter_hwnd(window).is_some_and(covered)
    }

    fn covered(meter: isize) -> bool {
        let mut state = 0i32;
        // SAFETY: `state` is a valid out-pointer.
        if unsafe { SHQueryUserNotificationState(&mut state) } == 0
            && state == QUNS_RUNNING_D3D_FULL_SCREEN
        {
            return true;
        }
        // SAFETY: window queries only.
        let monitor = unsafe { MonitorFromWindow(meter, MONITOR_DEFAULTTONEAREST) };
        let Some(rect) = monitor_rect(monitor) else {
            return false;
        };
        let taskbar = taskbar_on(monitor);
        if taskbar == 0 {
            // No taskbar to compare against: fall back to the foreground.
            // SAFETY: window queries only.
            let fg = unsafe { GetForegroundWindow() };
            return fg != 0
                && fg != meter
                && unsafe { MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST) } == monitor
                && covers_monitor(fg, rect);
        }
        // SAFETY: z-order walk over top-level windows; read-only queries.
        unsafe {
            let mut hwnd = GetTopWindow(0);
            for _ in 0..MAX_WALK {
                if hwnd == 0 || hwnd == taskbar {
                    return false;
                }
                if hwnd != meter && covers_monitor(hwnd, rect) {
                    return true;
                }
                hwnd = GetWindow(hwnd, GW_HWNDNEXT);
            }
        }
        false
    }

    static MOUSE_HOOKED: AtomicBool = AtomicBool::new(false);
    /// Primary press on the meter swallowed by the hook, not yet released.
    static HOOK_BUTTON_DOWN: AtomicBool = AtomicBool::new(false);
    /// Secondary press on the meter swallowed by the hook.
    static HOOK_RIGHT_DOWN: AtomicBool = AtomicBool::new(false);

    const WH_MOUSE_LL: i32 = 14;
    const WM_LBUTTONDOWN: usize = 0x0201;
    const WM_LBUTTONUP: usize = 0x0202;
    const WM_RBUTTONDOWN: usize = 0x0204;
    const WM_RBUTTONUP: usize = 0x0205;

    #[repr(C)]
    struct MouseHookInfo {
        pt: Point,
        mouse_data: u32,
        flags: u32,
        time: u32,
        extra: usize,
    }

    #[repr(C)]
    struct Msg {
        hwnd: isize,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: Point,
        private: u32,
    }

    type MouseProc = unsafe extern "system" fn(i32, usize, isize) -> isize;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn SetWindowsHookExW(id: i32, proc: MouseProc, module: isize, thread: u32) -> isize;
        fn CallNextHookEx(hook: isize, code: i32, wparam: usize, lparam: isize) -> isize;
        fn GetMessageW(msg: *mut Msg, hwnd: isize, min: u32, max: u32) -> i32;
    }

    fn over_meter(pt: &Point) -> bool {
        let meter = METER_HWND.load(Ordering::Acquire);
        if meter == 0 {
            return false;
        }
        let mut rect = Rect::default();
        // SAFETY: window queries on our own window with a valid out-pointer.
        unsafe {
            IsWindowVisible(meter) != 0
                && GetWindowRect(meter, &mut rect) != 0
                && pt.x >= rect.left
                && pt.x < rect.right
                && pt.y >= rect.top
                && pt.y < rect.bottom
        }
    }

    /// Embedded in the taskbar, the meter's webview never gets mouse input:
    /// WebView2's input window is click-through (WS_EX_TRANSPARENT) and hit
    /// testing falls through to the taskbar in another process. So presses
    /// over the meter are handled here and kept from the taskbar.
    unsafe extern "system" fn on_mouse(code: i32, wparam: usize, lparam: isize) -> isize {
        if code >= 0 && EMBEDDED.load(Ordering::Acquire) {
            // SAFETY: for WH_MOUSE_LL, lparam points to an MSLLHOOKSTRUCT.
            let info = unsafe { &*(lparam as *const MouseHookInfo) };
            match wparam {
                WM_LBUTTONUP if HOOK_BUTTON_DOWN.swap(false, Ordering::AcqRel) => return 1,
                WM_RBUTTONUP if HOOK_RIGHT_DOWN.swap(false, Ordering::AcqRel) => {
                    super::hook_context_menu();
                    return 1;
                }
                WM_LBUTTONDOWN if over_meter(&info.pt) => {
                    HOOK_BUTTON_DOWN.store(true, Ordering::Release);
                    super::hook_press();
                    return 1;
                }
                WM_RBUTTONDOWN if over_meter(&info.pt) => {
                    HOOK_RIGHT_DOWN.store(true, Ordering::Release);
                    return 1;
                }
                _ => {}
            }
        }
        // SAFETY: forwarding the event unchanged.
        unsafe { CallNextHookEx(0, code, wparam, lparam) }
    }

    /// Install the low-level mouse hook once, on its own thread with its own
    /// message loop, so a busy UI thread can never stall system-wide input.
    pub fn install_mouse_hook() {
        if MOUSE_HOOKED.swap(true, Ordering::AcqRel) {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name("taskbar-meter-mouse".into())
            .spawn(|| {
                // SAFETY: the hook procedure has the LowLevelMouseProc
                // signature; GetMessageW pumps this thread's queue (required
                // for low-level hooks) with a valid MSG out-pointer.
                unsafe {
                    if SetWindowsHookExW(WH_MOUSE_LL, on_mouse, 0, 0) == 0 {
                        tracing::warn!("taskbar meter: mouse hook failed");
                        return;
                    }
                    let mut msg = std::mem::zeroed::<Msg>();
                    while GetMessageW(&mut msg, 0, 0, 0) > 0 {}
                }
            });
        if spawned.is_err() {
            MOUSE_HOOKED.store(false, Ordering::Release);
        }
    }

    /// Cursor position in physical screen pixels.
    pub fn cursor_position() -> Option<(i32, i32)> {
        let mut point = Point::default();
        // SAFETY: `point` is a valid out-pointer.
        (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
    }

    /// Whether the primary mouse button is physically held right now.
    pub fn primary_button_down() -> bool {
        if EMBEDDED.load(Ordering::Acquire) {
            // The mouse hook swallowed the press, so the system (and
            // GetAsyncKeyState) never saw it; the hook tracks the release.
            return HOOK_BUTTON_DOWN.load(Ordering::Acquire);
        }
        // SAFETY: stateless input queries.
        unsafe {
            // GetAsyncKeyState reads physical buttons; honor swapped buttons.
            let key = if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
                VK_RBUTTON
            } else {
                VK_LBUTTON
            };
            GetAsyncKeyState(key) < 0
        }
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn taskbar_is_covered(_window: &tauri::WebviewWindow) -> bool {
        false
    }
    pub fn is_embedded() -> bool {
        false
    }
    pub fn taskbar_exists() -> bool {
        true
    }
    pub fn embed(_window: &tauri::WebviewWindow, _point: (i32, i32)) -> Option<super::Strip> {
        None
    }
    pub fn host_rect(_window: &tauri::WebviewWindow) -> Option<super::Strip> {
        None
    }
    pub fn move_child(_window: &tauri::WebviewWindow, _x: i32, _y: i32) {}
    pub fn set_shown(window: &tauri::WebviewWindow, shown: bool) {
        let _changed = if shown { window.show() } else { window.hide() };
    }
    pub fn cursor_position() -> Option<(i32, i32)> {
        None
    }
    pub fn primary_button_down() -> bool {
        false
    }
    pub fn track_window(_window: &tauri::WebviewWindow) {}
    pub fn forget_window() {}
    pub fn install_zorder_hook() {}
    pub fn install_mouse_hook() {}
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
