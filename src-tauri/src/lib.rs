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

            // 在所有 Space 可见：否则激活时 macOS 会把当前 Space 切到窗口那屏，
            // 多显示器下表现成"两块屏先后亮一下"。
            if let Some(panel) = app.get_webview_window("panel") {
                let _ = panel.set_visible_on_all_workspaces(true);
            }

            tray::build(&handle)?;

            if let Err(e) = handle.global_shortcut().register(panel_shortcut()) {
                eprintln!("[jcopy] 全局快捷键注册失败（可能被其他应用占用）: {e}");
            }

            clipboard::spawn(handle.clone(), state.clone());
            sync::engine::spawn(handle.clone(), state.clone());
            sync::lan::spawn(handle.clone(), state);

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
            commands::probe_report,
            commands::set_panel_mode,
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
        std::thread::sleep(std::time::Duration::from_millis(3500));
        // 走 IPC 回传：eval 没有返回值，而读 window.url() 在 webview 尚未加载时
        // 会命中 wry 里的 unwrap(nil) 直接把主线程打崩。
        let script = r#"(() => {
            const root = document.documentElement;
            const s = document.querySelector('.surface');
            const rail = document.querySelector('.rail');
            const cards = document.querySelectorAll('.card');
            const report = (info) => window.__TAURI_INTERNALS__.invoke('probe_report', { info });

            let base = 'NO_SURFACE';
            if (s) {
                const r = s.getBoundingClientRect();
                base = Math.round(r.width) + 'x' + Math.round(r.height)
                    + '-cards-' + cards.length
                    + '-theme=' + root.dataset.theme
                    + '-font=' + root.dataset.font
                    + '-scrollLeft=' + (rail ? rail.scrollLeft : -1);
            }

            // 点齿轮打开设置弹窗：确认它是叠层，抽屉高度不被撑高
            const gear = document.querySelector('.icon-btn');
            if (!gear) {
                report(base + ' | noGear');
                return;
            }
            gear.click();
            requestAnimationFrame(() => {
                const modal = document.querySelector('.modal');
                const size = modal
                    ? Math.round(modal.getBoundingClientRect().width) + 'x' + Math.round(modal.getBoundingClientRect().height)
                    : 'NONE';
                const winH = s ? Math.round(s.getBoundingClientRect().height) : -1;
                report(base + ' | modal=' + size
                    + ' segs=' + document.querySelectorAll('.seg').length
                    + ' diag=' + document.querySelectorAll('.sync-diag').length
                    + ' rows=' + document.querySelectorAll('.diag-line').length
                    + ' winH=' + winH);
                const done = document.querySelector('.modal .ghost');
                if (done) done.click();
            });

            // 同步自检：等一轮 push/pull 之后再查，否则数据还没写出来
            setTimeout(() => {
                window.__TAURI_INTERNALS__.invoke('sync_status').then((st) => {
                    const files = (st.peerFiles || []).map((f) => {
                        const tag = f.isSelf ? 'self' : f.id.slice(0, 6);
                        return tag + ':' + (f.decryptable ? f.clips + '条' : f.stale ? 'STALE' : 'BAD');
                    }).join(',');
                    report('sync fp=' + (st.keyFingerprint || '-') + ' own=' + st.ownClips
                        + ' files=[' + files + '] err=' + (st.lastError || '-'));
                }).catch((e) => report('sync invoke 失败: ' + e));
            }, 4200);

            // 模拟在搜索框里打字：验证延迟过滤 + 高亮仍然工作
            setTimeout(() => {
                const input = document.querySelector('input');
                if (!input) {
                    report('search: 没有输入框');
                    return;
                }
                const before = document.querySelectorAll('.card').length;
                const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
                setter.call(input, 'Node');
                input.dispatchEvent(new Event('input', { bubbles: true }));
                setTimeout(() => {
                    report('search "' + input.value + '" cards ' + before + '→'
                        + document.querySelectorAll('.card').length
                        + ' marks=' + document.querySelectorAll('mark').length);
                }, 400);
            }, 900);
        })()"#;
        let _ = window.eval(script);
    });
}
