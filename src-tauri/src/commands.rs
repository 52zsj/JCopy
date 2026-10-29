use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

use crate::model::{ClipView, Settings};
use crate::state::{save_settings, AppState};
use crate::{db, platform};

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub settings: Settings,
    pub clips: Vec<ClipView>,
    pub paused: bool,
    pub can_paste: bool,
}

#[tauri::command]
pub fn bootstrap(state: State<'_, Arc<AppState>>) -> Result<Bootstrap, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?.clone();
    let clips = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::list(&conn, 500)
            .map_err(|e| e.to_string())?
            .iter()
            .map(|c| c.view())
            .collect()
    };
    Ok(Bootstrap {
        settings,
        clips,
        paused: state.paused.load(Ordering::Relaxed),
        can_paste: platform::accessible(),
    })
}

#[tauri::command]
pub fn list_clips(state: State<'_, Arc<AppState>>) -> Result<Vec<ClipView>, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    Ok(db::list(&conn, 500)
        .map_err(|e| e.to_string())?
        .iter()
        .map(|c| c.view())
        .collect())
}

/// 把选中条目写回剪贴板；autoPaste 开启时同时向原应用注入粘贴快捷键。
#[tauri::command]
pub fn copy_clip(app: AppHandle, state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    let auto_paste = {
        state.settings.lock().map_err(|e| e.to_string())?.auto_paste
    };

    let clip = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::get(&conn, &id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "该条目已不存在".to_string())?
    };

    {
        let mut cb = arboard::Clipboard::new().map_err(|e| format!("剪贴板不可用：{e}"))?;
        cb.set_text(clip.content.clone())
            .map_err(|e| format!("写入剪贴板失败：{e}"))?;
    }

    // 让监听线程忽略这次变化，否则会立刻生成一条"来源=自己"的重复记录
    *state.ignore_hash.lock().unwrap() = Some(clip.hash.clone());

    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        let _ = db::touch_used(&conn, &id);
    }

    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.hide();
    }

    if auto_paste {
        // 给窗口隐藏与焦点归还原应用留出时间
        std::thread::sleep(Duration::from_millis(130));
        platform::paste()?;
    }
    Ok(())
}

#[tauri::command]
pub fn delete_clip(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::delete(&conn, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_all(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::clear(&conn).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn toggle_pin(state: State<'_, Arc<AppState>>, id: String) -> Result<bool, String> {
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::toggle_pin(&conn, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_settings(
    state: State<'_, Arc<AppState>>,
    settings: Settings,
) -> Result<Settings, String> {
    let mut current = state.settings.lock().map_err(|e| e.to_string())?;
    *current = settings;
    save_settings(&state.config_dir, &current);
    Ok(current.clone())
}

#[tauri::command]
pub fn set_paused(state: State<'_, Arc<AppState>>, paused: bool) -> Result<bool, String> {
    state.paused.store(paused, Ordering::Relaxed);
    Ok(paused)
}

#[tauri::command]
pub fn hide_panel(app: AppHandle) {
    if let Some(w) = app.get_webview_window("panel") {
        let _ = w.hide();
    }
}

/// macOS 下直接跳到「隐私与安全性 → 辅助功能」，省去用户找路径。
#[tauri::command]
pub fn open_accessibility_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

#[tauri::command]
pub fn open_config_dir(state: State<'_, Arc<AppState>>) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(&state.config_dir).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg(&state.config_dir).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(&state.config_dir).spawn();
}
