use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewWindow};

/// 最近一次显示面板的时间戳，用于忽略"刚显示就失焦"的假事件
static LAST_SHOWN_MS: AtomicU64 = AtomicU64::new(0);

/// 横向卡片条的高度（逻辑像素）：工具栏 + 一行卡片
const CLIPS_HEIGHT: f64 = 268.0;
/// 设置页内容多，临时把抽屉拉高
const SETTINGS_HEIGHT: f64 = 480.0;

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
    layout_drawer(&w, CLIPS_HEIGHT);
    let _ = w.show();
    #[cfg(target_os = "macos")]
    activate_app();
    let _ = w.set_focus();
    LAST_SHOWN_MS.store(now_ms(), Ordering::Relaxed);
    let _ = app.emit("panel-shown", ());
}

/// 切换面板高度：剪贴板是扁条，设置页需要更多纵向空间。
pub fn set_mode(app: &AppHandle, mode: &str) {
    let Some(w) = app.get_webview_window("panel") else {
        return;
    };
    let height = if mode == "settings" {
        SETTINGS_HEIGHT
    } else {
        CLIPS_HEIGHT
    };
    layout_drawer(&w, height);
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

/// 底部抽屉：宽度按屏幕走，紧贴屏幕底边，左右居中。
/// 每次显示都重算，换分辨率或挪到副屏之后不会跑到屏幕外。
fn layout_drawer(w: &WebviewWindow, height: f64) {
    let Ok(Some(monitor)) = w.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let origin = monitor.position();
    let screen = monitor.size();

    // 横向卡片条：尽量铺开，左右各留 40pt
    let screen_logical_w = screen.width as f64 / scale;
    let width = (screen_logical_w - 80.0).clamp(900.0, 1600.0);
    let _ = w.set_size(LogicalSize::new(width, height));

    let phys_w = (width * scale).round() as i32;
    let phys_h = (height * scale).round() as i32;
    let x = origin.x + (screen.width as i32 - phys_w) / 2;
    // 贴底：不留任何下边距
    let y = origin.y + screen.height as i32 - phys_h;

    let _ = w.set_position(PhysicalPosition::new(x.max(origin.x), y.max(origin.y)));
}
