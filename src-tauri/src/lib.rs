mod clipboard;
mod commands;
mod db;
mod model;
mod platform;
mod state;
mod sync;
mod tray;
mod ui;

use std::sync::Arc;
use tauri::Manager;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        ui::toggle(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();

            let state = Arc::new(state::init(&handle)?);
            app.manage(state.clone());

            // 菜单栏常驻：无 Dock 图标
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            tray::build(&handle)?;

            if let Err(e) = handle.global_shortcut().register(panel_shortcut()) {
                eprintln!("[jcopy] 全局快捷键注册失败（可能被其他应用占用）: {e}");
            }

            clipboard::spawn(handle.clone(), state.clone());
            sync::engine::spawn(handle.clone(), state);

            // 调试开关：启动即展开面板，省得每次按快捷键
            if std::env::var("JCOPY_SHOW_ON_START").is_ok() {
                let handle_for_show = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    ui::show(&handle_for_show);
                });
                #[cfg(debug_assertions)]
                probe_dom(&handle);
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::list_clips,
            commands::copy_clip,
            commands::delete_clip,
            commands::clear_all,
            commands::toggle_pin,
            commands::update_settings,
            commands::set_paused,
            commands::hide_panel,
            commands::open_accessibility_settings,
            commands::open_config_dir,
            commands::sync_status,
            commands::sync_set_dir,
            commands::sync_set_enabled,
            commands::sync_create_invite,
            commands::sync_join,
            commands::sync_now,
            commands::sync_reset,
        ])
        .on_window_event(|window, event| match event {
            // 点面板外任意位置即收起，这是剪贴板工具的标配行为
            tauri::WindowEvent::Focused(false) => {
                if ui::recently_shown() {
                    return;
                }
                if std::env::var("JCOPY_NO_AUTOHIDE").is_err() {
                    let _ = window.hide();
                }
            }
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("JCopy 初始化失败")
        .run(|_app, event| {
            // 关窗不退出，常驻菜单栏
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}

fn panel_shortcut() -> Shortcut {
    #[cfg(target_os = "macos")]
    let modifiers = Modifiers::SUPER | Modifiers::SHIFT;
    #[cfg(not(target_os = "macos"))]
    let modifiers = Modifiers::CONTROL | Modifiers::SHIFT;

    Shortcut::new(Some(modifiers), Code::KeyV)
}

/// 仅调试用：读取面板的真实渲染尺寸，确认 CSS 与列表都正常。
#[cfg(debug_assertions)]
fn probe_dom(app: &tauri::AppHandle) {
    let Some(window) = app.get_webview_window("panel") else {
        return;
    };
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(2600));
        // 用 hash 回传结果：eval 没有返回值，而 hash 变化不需要导航
        let script = r#"(() => {
            const s = document.querySelector('.surface');
            const n = document.querySelectorAll('.item').length;
            let out = 'NO_SURFACE';
            if (s) {
                const r = s.getBoundingClientRect();
                out = Math.round(r.width) + 'x' + Math.round(r.height) + '-items-' + n;
            }
            location.hash = 'probe=' + out;
        })()"#;
        let _ = window.eval(script);
        std::thread::sleep(std::time::Duration::from_millis(800));
        let frag = window
            .url()
            .ok()
            .and_then(|u| u.fragment().map(|f| f.to_string()))
            .unwrap_or_default();
        eprintln!("[jcopy] DOM 探针: {frag}");
    });
}
