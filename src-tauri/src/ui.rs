// cocoa crate 已被官方标记废弃（建议迁 objc2），但本文件还在用它调 NSWindow 的
// setFrame / 激活策略。迁移是独立任务，这里先局部静音，免得 macOS 构建刷屏
// 把真正有用的警告埋掉；其它模块的废弃警告仍然可见。
#![allow(deprecated)]

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
    // 隐藏状态下移动窗口时，WindowServer 要到下一帧才真正提交位置。
    // 不等这一帧就 show，会在旧屏幕闪一下再跳过去——多屏时尤其明显。
    std::thread::sleep(std::time::Duration::from_millis(24));
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
///
/// 顺序很关键：先把窗口缩到最小、再定位、最后设最终尺寸。
/// 反过来的话（先 set_size 后 set_position），当两块屏宽度不同时，
/// 窗口会带着目标屏的宽度停在原来那块屏的位置上，横跨两块屏各露出半截，
/// 视觉上就是"先闪一下、缩一下、再跑到另一块屏"。
fn layout_drawer(w: &WebviewWindow, height: f64) {
    #[cfg(target_os = "macos")]
    if let Some(screen) = mac_screen_under_mouse() {
        let y_top = screen.y + screen.h - height;
        // 优先直接改 NSWindow 的 frame：位置和尺寸一次设定，不存在中间态
        if place_with_ns_window(w, screen.x, y_top, screen.w, height, screen.primary_top) {
            return;
        }
        resize_and_place(w, screen.x, y_top, screen.w, height);
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

    resize_and_place(w, logical_x, logical_y + logical_h - height, logical_w, height);
}

/// 直接对 NSWindow 调 `setFrame:display:`。
///
/// Tauri 的 `set_size` / `set_position` 是投递到主线程的两次独立调用，中间存在
/// "新尺寸 + 旧位置"的中间态；两块屏宽度不同时，窗口横跨两块屏各露半截，
/// 看起来就是先闪一下再缩一下才跑到目标屏。一次 setFrame 没有这个中间态。
///
/// AppKit 的几何操作必须在主线程，所以先 `run_on_main_thread` 投递，
/// 再在 channel 上等它做完——否则 `show()` 会抢在 setFrame 前面。
#[cfg(target_os = "macos")]
fn place_with_ns_window(
    w: &WebviewWindow,
    x: f64,
    y_top: f64,
    width: f64,
    height: f64,
    primary_top: f64,
) -> bool {
    let Ok(ptr) = w.ns_window() else {
        return false;
    };
    if ptr.is_null() {
        return false;
    }

    let address = ptr as usize;
    let (tx, rx) = std::sync::mpsc::channel::<()>();

    let dispatched = w.run_on_main_thread(move || {
        use cocoa::base::id;
        use cocoa::foundation::{NSPoint, NSRect, NSSize};
        use objc::{msg_send, sel, sel_impl};

        let ns_window = address as id;
        unsafe {
            // NSWindow 的 frame 是 Cocoa 坐标（左下原点、y 向上）
            let y_cocoa = primary_top - (y_top + height);
            // 先塌到目标屏底部一个 8×8 的点：即便中间被渲染了一帧，也只出现在目标屏的角落
            let tiny = NSRect::new(NSPoint::new(x, y_cocoa), NSSize::new(8.0, 8.0));
            let _: () = msg_send![ns_window, setFrame: tiny display: false];

            let frame = NSRect::new(NSPoint::new(x, y_cocoa), NSSize::new(width, height));
            let _: () = msg_send![ns_window, setFrame: frame display: false];
        }
        let _ = tx.send(());
    });

    if dispatched.is_err() {
        return false;
    }
    // 等主线程真的做完，才允许继续 show()
    rx.recv_timeout(std::time::Duration::from_millis(300)).is_ok()
}

/// 最小 → 定位 → 最终尺寸。窗口此刻是隐藏的，中间的极小尺寸不可见。
fn resize_and_place(w: &WebviewWindow, x: f64, y: f64, width: f64, height: f64) {
    // 1. 缩到几乎不可见，这样接下来的跨屏移动不会在两块屏上各露一截
    let _ = w.set_size(LogicalSize::new(8.0, 8.0));
    // 2. 移到目标位置
    let _ = w.set_position(LogicalPosition::new(x, y));
    // 3. 给出最终尺寸
    let _ = w.set_size(LogicalSize::new(width, height));
    // 4. 读一次几何作为屏障：这些调用是投递到主线程的，读操作会排在后面，
    //    保证 show() 之前 frame 已经真正生效
    let _ = w.inner_size();
    let _ = w.outer_position();
}

#[cfg(target_os = "macos")]
struct ScreenRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    /// 主屏顶部在 Cocoa 坐标里的 y（用于把"左上原点"换算回去）
    primary_top: f64,
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
                primary_top: primary_frame.origin.y + primary_frame.size.height,
            });
        }
        None
    }
}
