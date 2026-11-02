use std::fs;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

use crate::model::{ClipView, Settings};
use crate::state::{save_settings, AppState, PeerStatus};
use crate::sync::{crypto, engine, protocol, protocol::InvitePayload};
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
            .collect::<Vec<_>>()
    };
    eprintln!(
        "[jcopy] 前端已就绪：{} 条历史，设备「{}」，自动粘贴={}",
        clips.len(),
        settings.device_name,
        settings.auto_paste
    );
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
        if clip.kind == "image" {
            // content 是原图路径，读回来解码成 RGBA 再塞进剪贴板
            let img = image::open(&clip.content)
                .map_err(|e| format!("图片读取失败：{e}"))?
                .to_rgba8();
            let (width, height) = img.dimensions();
            let data = arboard::ImageData {
                width: width as usize,
                height: height as usize,
                bytes: std::borrow::Cow::Owned(img.into_raw()),
            };
            cb.set_image(data)
                .map_err(|e| format!("写入剪贴板失败：{e}"))?;
        } else {
            cb.set_text(clip.content.clone())
                .map_err(|e| format!("写入剪贴板失败：{e}"))?;
        }
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

/// 仅调试用：把面板的渲染尺寸回传出来，供无头验证（截图受限时的唯一手段）。
#[tauri::command]
pub fn probe_report(info: String) {
    #[cfg(debug_assertions)]
    eprintln!("[jcopy] DOM 探针: {info}");
    #[cfg(not(debug_assertions))]
    let _ = info;
}

/// 切换抽屉高度：卡片模式是扁条，设置页需要更高。
#[tauri::command]
pub fn set_panel_mode(app: AppHandle, mode: String) {
    crate::ui::set_mode(&app, &mode);
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

// ---------------------------------------------------------------- 跨设备同步

/// 共享目录里的一个发件箱文件。用来回答"文件都在，为什么数据没过来"。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeerFile {
    /// 设备 id（文件名去掉 .enc）
    pub id: String,
    pub size: u64,
    pub updated_at: i64,
    /// 是不是本机写的
    pub is_self: bool,
    /// 能否用本机主密钥解开——false 基本等于"两台机器的配对密钥不一致"
    pub decryptable: bool,
    /// 能解开时，里面有多少条待同步内容
    pub clips: usize,
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    pub paired: bool,
    pub dir: String,
    pub dir_exists: bool,
    pub is_icloud: bool,
    pub device_id: String,
    pub device_name: String,
    pub peers: Vec<PeerStatus>,
    pub last_push_at: i64,
    pub last_pull_at: i64,
    pub last_error: Option<String>,
    /// 同网段发现的设备数（局域网直连可用性）
    pub lan_peers: usize,
    /// 主密钥指纹，两台机器对比即可判断是否配到同一把密钥
    pub key_fingerprint: Option<String>,
    /// 本机发件箱里实际发了多少条（0 说明过滤后没东西可发）
    pub own_clips: usize,
    /// 共享目录下的全部发件箱文件（含自己那份）
    pub peer_files: Vec<PeerFile>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfo {
    pub code: String,
    pub expires_at: i64,
    pub path: String,
}

fn current_sync_dir(state: &AppState) -> PathBuf {
    state
        .sync
        .lock()
        .ok()
        .and_then(|rt| rt.dir.clone())
        .unwrap_or_else(crate::state::default_sync_dir)
}

fn decode_b64_key(b64: &str) -> Option<crypto::Key> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;
    let bytes = B64.decode(b64).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Some(key)
}

#[tauri::command]
pub fn sync_status(state: State<'_, Arc<AppState>>) -> Result<SyncStatus, String> {
    let settings = state.settings.lock().map_err(|e| e.to_string())?.clone();
    let dir = current_sync_dir(&state);
    let is_icloud = crate::state::detect_icloud_dir()
        .map(|d| dir.starts_with(&d))
        .unwrap_or(false);

    // 先取 lan 再取 sync：广播侧是 lan → sync 的顺序，反过来会死锁
    let lan_peers = state.lan.lock().map(|l| l.peers.len()).unwrap_or(0);

    let (enabled, key, last_push_at, last_pull_at, last_error, peers) = {
        let rt = state.sync.lock().map_err(|e| e.to_string())?;
        let mut peers: Vec<PeerStatus> = rt.peers.values().cloned().collect();
        peers.sort_by(|a, b| a.name.cmp(&b.name));
        (
            rt.enabled,
            rt.key,
            rt.last_push_at,
            rt.last_pull_at,
            rt.last_error.clone(),
            peers,
        )
    };

    // 扫一遍共享目录：每个发件箱能不能解开、里面有多少条。
    // 这是区分「没配对成功」和「配对成功但没东西可发」的唯一可靠办法。
    let mut peer_files: Vec<PeerFile> = Vec::new();
    let devices_dir = dir.join("JCopy").join("devices");
    if let Ok(entries) = fs::read_dir(&devices_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("enc") {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
                continue;
            };

            let meta = entry.metadata().ok();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let updated_at = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);

            let is_self = id == settings.device_id;
            let mut decryptable = false;
            let mut clips = 0usize;
            let mut error = None;

            match (key, fs::read(&path)) {
                (None, _) => error = Some("本机还没有主密钥（未配对）".into()),
                (Some(k), Ok(blob)) => match protocol::open_outbox(&k, &id, &blob) {
                    Some(outbox) => {
                        decryptable = true;
                        clips = outbox.clips.len();
                    }
                    None if is_self => error = Some("本机发件箱解不开（密钥或文件异常）".into()),
                    None => error = Some("解不开：这台机器的主密钥与对方不一致".into()),
                },
                (Some(_), Err(err)) => error = Some(format!("读取失败：{err}")),
            }

            peer_files.push(PeerFile {
                id,
                size,
                updated_at,
                is_self,
                decryptable,
                clips,
                error,
            });
        }
    }
    peer_files.sort_by(|a, b| a.id.cmp(&b.id));
    let own_clips = peer_files
        .iter()
        .find(|f| f.is_self)
        .map(|f| f.clips)
        .unwrap_or(0);

    Ok(SyncStatus {
        enabled,
        paired: key.is_some(),
        dir: dir.display().to_string(),
        dir_exists: dir.is_dir(),
        is_icloud,
        device_id: settings.device_id,
        device_name: settings.device_name,
        peers,
        last_push_at,
        last_pull_at,
        last_error,
        lan_peers,
        key_fingerprint: key.map(|k| crypto::fingerprint(&k)),
        own_clips,
        peer_files,
    })
}

#[tauri::command]
pub fn sync_set_dir(state: State<'_, Arc<AppState>>, path: String) -> Result<(), String> {
    let dir = PathBuf::from(path.trim());
    if !dir.is_dir() {
        return Err("目录不存在".into());
    }
    {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        s.sync_dir = Some(dir.display().to_string());
        save_settings(&state.config_dir, &s);
    }
    let mut rt = state.sync.lock().map_err(|e| e.to_string())?;
    rt.dir = Some(dir);
    rt.last_push_hash.clear();
    Ok(())
}

#[tauri::command]
pub fn sync_set_enabled(state: State<'_, Arc<AppState>>, enabled: bool) -> Result<(), String> {
    {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        s.sync_enabled = enabled;
        save_settings(&state.config_dir, &s);
    }
    let mut rt = state.sync.lock().map_err(|e| e.to_string())?;
    rt.enabled = enabled;
    Ok(())
}

/// 生成一次性配对邀请：写进共享目录，用 6 位配对码加密主密钥。
/// 不依赖局域网，两台设备不在同一网络也能配对。
#[tauri::command]
pub fn sync_create_invite(state: State<'_, Arc<AppState>>) -> Result<InviteInfo, String> {
    use base64::engine::general_purpose::STANDARD as B64;
    use base64::Engine;

    let key = {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        match s.sync_key.as_deref().and_then(decode_b64_key) {
            Some(k) => k,
            None => {
                let k = crypto::random_key();
                s.sync_key = Some(B64.encode(k));
                save_settings(&state.config_dir, &s);
                k
            }
        }
    };
    {
        let mut rt = state.sync.lock().map_err(|e| e.to_string())?;
        rt.key = Some(key);
    }

    let settings = state.settings.lock().map_err(|e| e.to_string())?.clone();
    let dir = current_sync_dir(&state);
    if !dir.is_dir() {
        return Err(format!("同步目录不可用：{}", dir.display()));
    }

    let code = crypto::random_pair_code();
    let payload = InvitePayload {
        key: B64.encode(key),
        device_id: settings.device_id.clone(),
        device_name: settings.device_name.clone(),
    };

    let mut invite = protocol::seal_invite(&code, &payload).ok_or("生成配对邀请失败")?;
    invite.expires_at = protocol::db_now_ms() + 10 * 60 * 1000;

    let pair_dir = dir.join("JCopy").join("pair");
    fs::create_dir_all(&pair_dir).map_err(|e| e.to_string())?;
    let file = pair_dir.join(format!("{}.invite", invite.invite_id));
    let raw = serde_json::to_vec_pretty(&invite).map_err(|e| e.to_string())?;
    fs::write(&file, raw).map_err(|e| e.to_string())?;

    Ok(InviteInfo {
        code,
        expires_at: invite.expires_at,
        path: file.display().to_string(),
    })
}

/// 用配对码加入：扫描共享目录里的邀请文件并逐个试解。
#[tauri::command]
pub fn sync_join(state: State<'_, Arc<AppState>>, code: String) -> Result<String, String> {
    let code = code.trim().to_string();
    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        return Err("配对码是 6 位数字".into());
    }

    let dir = current_sync_dir(&state);
    let pair_dir = dir.join("JCopy").join("pair");
    let entries = fs::read_dir(&pair_dir).map_err(|_| {
        "同步目录里没有找到配对邀请。请确认两台设备登录的是同一个 iCloud 账号，且已装 iCloud 云盘客户端"
            .to_string()
    })?;

    let mut found: Option<(PathBuf, InvitePayload)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("invite") {
            continue;
        }
        let Ok(raw) = fs::read(&path) else { continue };
        let Ok(invite) = serde_json::from_slice::<protocol::Invite>(&raw) else {
            continue;
        };
        if let Some(payload) = protocol::open_invite(&code, &invite) {
            found = Some((path, payload));
            break;
        }
    }

    let Some((path, payload)) = found else {
        return Err("配对码不正确，或邀请已过期".into());
    };

    let key = decode_b64_key(&payload.key).ok_or("邀请内容损坏")?;
    {
        let mut rt = state.sync.lock().map_err(|e| e.to_string())?;
        rt.key = Some(key);
        rt.enabled = true;
        rt.last_push_hash.clear();
    }
    {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        s.sync_key = Some(payload.key.clone());
        s.sync_enabled = true;
        save_settings(&state.config_dir, &s);
    }

    // 邀请是一次性的
    let _ = fs::remove_file(path);
    Ok(payload.device_name)
}

#[tauri::command]
pub fn sync_now(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let inner = state.inner().clone();
    engine::sync_once(&app, &inner)
}

#[tauri::command]
pub fn sync_reset(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    {
        let mut s = state.settings.lock().map_err(|e| e.to_string())?;
        s.sync_key = None;
        save_settings(&state.config_dir, &s);
    }
    let mut rt = state.sync.lock().map_err(|e| e.to_string())?;
    rt.key = None;
    rt.peers.clear();
    rt.last_push_hash.clear();
    Ok(())
}
