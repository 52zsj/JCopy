use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewWindow};

/// 最近一次显示面板的时间戳，用于忽略"刚显示就失焦"的假事件
static LAST_SHOWN_MS: AtomicU64 = AtomicU64::new(0);

/// 横向卡片条的高度（逻辑像素）：工具栏 + 一行卡片
const CLIPS_HEIGHT: f64 = 380.0;
/// 设置页内容多，临时把抽屉拉高
const SETTINGS_HEIGHT: f64 = 560.0;

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

/// 底部抽屉：铺满当前屏幕宽度、紧贴屏幕底边。
/// 屏幕由鼠标位置决定——连续用副屏时不必先点一下窗口再按快捷键。
fn layout_drawer(w: &WebviewWindow, height: f64) {
    #[cfg(target_os = "macos")]
    if let Some(screen) = mac_screen_under_mouse() {
        let _ = w.set_size(LogicalSize::new(screen.w, height));
        let y = screen.y + screen.h - height;
        let _ = w.set_position(LogicalPosition::new(screen.x, y));
        return;
    }

    // 非 macOS 或取不到鼠标屏幕时，退回"窗口当前所在屏"
    let Ok(Some(monitor)) = w.current_monitor() else {
        return;
    };
    let scale = monitor.scale_factor();
    let size = monitor.size();
    let origin = monitor.position();
    let logical_w = size.width as f64 / scale;
    let logical_x = origin.x as f64 / scale;
    let logical_y = origin.y as f64 / scale;
    let logical_h = size.height as f64 / scale;

    let _ = w.set_size(LogicalSize::new(logical_w, height));
    let _ = w.set_position(LogicalPosition::new(logical_x, logical_y + logical_h - height));
}

#[cfg(target_os = "macos")]
struct ScreenRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// 用 NSScreen + NSEvent.mouseLocation 找鼠标所在屏幕，返回"主屏左上"为原点的逻辑坐标。
/// 不走 Tauri 的 monitor 列表，是因为混合 DPI 时它的 position/size 单位不统一。
#[cfg(target_os = "macos")]
fn mac_screen_under_mouse() -> Option<ScreenRect> {
    use cocoa::base::{id, nil};
    use cocoa::foundation::{NSPoint, NSRect};
    use objc::{class, msg_send, sel, sel_impl};

    unsafe {
        let screens: id = msg_send![class!(NSScreen), screens];
        if screens == nil {
            return None;
        }
        let count: usize = msg_send![screens, count];
        if count == 0 {
            return None;
        }
        let primary: id = msg_send![screens, objectAtIndex: 0usize];
        let primary_frame: NSRect = msg_send![primary, frame];

        let mouse: NSPoint = msg_send![class!(NSEvent), mouseLocation];

        for index in 0..count {
            let screen: id = msg_send![screens, objectAtIndex: index];
            let frame: NSRect = msg_send![screen, frame];

            let inside = mouse.x >= frame.origin.x
                && mouse.x < frame.origin.x + frame.size.width
                && mouse.y >= frame.origin.y
                && mouse.y < frame.origin.y + frame.size.height;
            if !inside {
                continue;
            }

            // NSScreen 原点在左下、y 向上；窗口坐标要的是主屏左上、y 向下
            let x = frame.origin.x - primary_frame.origin.x;
            let y = (primary_frame.origin.y + primary_frame.size.height)
                - (frame.origin.y + frame.size.height);
            return Some(ScreenRect {
                x,
                y,
                w: frame.size.width,
                h: frame.size.height,
            });
        }
        None
    }
}
