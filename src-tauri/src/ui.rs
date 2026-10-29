use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

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
    if let Some(w) = app.get_webview_window("panel") {
        place_bottom_center(&w);
        let _ = w.show();
        let _ = w.set_focus();
        let _ = app.emit("panel-shown", ());
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
