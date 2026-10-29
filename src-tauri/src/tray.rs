use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示面板", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "暂停监听", true, None::<&str>)?;
    let clear = MenuItem::with_id(app, "clear", "清空历史…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 JCopy", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[&show, &pause, &sep1, &clear, &sep2, &quit])?;

    let mut builder = TrayIconBuilder::with_id("jcopy")
        .tooltip("JCopy")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => crate::ui::show(app),
            "pause" => {
                let state = app.state::<Arc<AppState>>();
                let next = !state.paused.load(Ordering::Relaxed);
                state.paused.store(next, Ordering::Relaxed);
            }
            "clear" => {
                let state = app.state::<Arc<AppState>>();
                {
                    let conn = state.db.lock().unwrap();
                    let _ = crate::db::clear(&conn);
                }
            }
            "quit" => std::process::exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                crate::ui::toggle(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}
