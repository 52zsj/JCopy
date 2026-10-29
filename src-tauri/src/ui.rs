use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

/// 最近一次显示面板的时间戳，用于忽略"刚显示就失焦"的假事件
static LAST_SHOWN_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn recently_shown() -> bool {
    now_ms().saturating_sub(LAST_SHOWN_MS.load(Ordering::Relaxed)) < 350
}

pub fn toggle(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("panel") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
        } else {
            show(app);
        }
    }
}

pub fn show(app: &AppHandle) {
    let Some(w) = app.get_webview_window("panel") else {
        eprintln!("[jcopy] 找不到 panel 窗口");
        return;
    };
    place_bottom_center(&w);
    let _ = w.show();
    #[cfg(target_os = "macos")]
    activate_app();
    let _ = w.set_focus();
    LAST_SHOWN_MS.store(now_ms(), Ordering::Relaxed);
    let _ = app.emit("panel-shown", ());
}

/// Accessory（无 Dock 图标）模式下，必须显式激活应用，否则窗口收不到键盘输入。
#[cfg(target_os = "macos")]
fn activate_app() {
    use cocoa::base::{id, nil};
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
        if ns_app != nil {
            let _: () = msg_send![ns_app, activateIgnoringOtherApps: true];
        }
    }
}

/// 贴在当前屏幕底部居中，留出 72pt 视觉呼吸区。
fn place_bottom_center(w: &WebviewWindow) {
    let Ok(Some(monitor)) = w.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let size = w
        .outer_size()
        .unwrap_or_else(|_| PhysicalSize::new(720, 520));
    let origin = monitor.position();
    let screen = monitor.size();

    let x = origin.x + (screen.width as i32 - size.width as i32) / 2;
    let y = origin.y + screen.height as i32 - size.height as i32 - (72.0 * scale) as i32;
    let _ = w.set_position(PhysicalPosition::new(x.max(origin.x), y.max(origin.y)));
}
