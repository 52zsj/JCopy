use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, State};

use crate::model::{ClipView, Settings};
use crate::state::{save_settings, AppState, PeerStatus};
use crate::sync::{engine, protocol};
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

/// 在系统文件管理器里定位这条记录的图片文件。文本条目没有对应文件。
#[tauri::command]
pub fn reveal_clip(state: State<'_, Arc<AppState>>, id: String) -> Result<(), String> {
    let clip = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::get(&conn, &id)
            .map_err(|e| e.to_string())?
            .ok_or("条目不存在")?
    };
    if clip.kind != "image" {
        return Err("只有图片条目有对应文件".into());
    }
    if !std::path::Path::new(&clip.content).is_file() {
        return Err("原图已被清理".into());
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &clip.content])
            .status()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "windows")]
    {
        // explorer /select, 成功时返回码也不是 0，所以不检查 status
        let _ = std::process::Command::new("explorer")
            .arg(format!("/select,{}", clip.content))
            .spawn();
    }

    #[cfg(target_os = "linux")]
    {
        let parent = std::path::Path::new(&clip.content)
            .parent()
            .ok_or("路径无效")?;
        std::process::Command::new("xdg-open")
            .arg(parent)
            .status()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
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
    pub updated_at: i64,
    /// 是不是本机写的
    pub is_self: bool,
    /// 能否用本机主密钥解开——false 基本等于"两台机器的配对密钥不一致"
    pub decryptable: bool,
    /// 能解开时，里面有多少条待同步内容
    pub clips: usize,
    /// 其中图片有几张
    pub images: usize,
    /// 图片条目里有几个原图文件在共享目录里找不到——
    /// 大于 0 说明卡在云盘传输，不是配对问题
    pub missing_images: usize,
    pub error: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub enabled: bool,
    pub dir: String,
    pub dir_exists: bool,
    pub is_icloud: bool,
    pub device_name: String,
    pub peers: Vec<PeerStatus>,
    pub last_pull_at: i64,
    pub last_error: Option<String>,
    /// 同网段发现的设备数（局域网直连可用性）
    /// 主密钥指纹，两台机器对比即可判断是否配到同一把密钥
    /// 本机发件箱里实际发了多少条（0 说明过滤后没东西可发）
    pub own_clips: usize,
    /// 最近一次发出去的内容：总条数 / 其中图片张数
    pub pushed_clips: usize,
    pub pushed_images: usize,
    /// 本次启动以来累计收到：总条数 / 其中图片张数
    pub pulled_clips: usize,
    pub pulled_images: usize,
    /// 有图片条目但原图还没落盘（正常应为 0）
    pub pending_images: usize,
    /// 对端有图、原图还没传过来（本机也还没有）→ 正在等云盘
    pub waiting_images: usize,
    /// 本机有内容超过体积上限被跳过（图片 12MB / 文本 256KB）
    pub skipped_oversize: usize,
    /// 共享目录里由本机放置的图片文件数（能直接对照 Windows 端有没有收到）
    pub shared_images: usize,
    /// 同步目录所在卷的剩余空间（MB）。iCloud 目录写满时文件写不进去，
    /// 但本地磁盘错误平时看不见，所以直接摆到界面上
    pub dir_free_mb: u64,
    /// 共享目录下的全部发件箱文件（含自己那份）；只有 deep 模式才扫描
    pub peer_files: Vec<PeerFile>,
}

fn current_sync_dir(state: &AppState) -> PathBuf {
    state
        .sync
        .lock()
        .ok()
        .and_then(|rt| rt.dir.clone())
        .or_else(crate::state::default_sync_dir)
        .unwrap_or_default()
}

#[tauri::command]
pub fn sync_status(
    state: State<'_, Arc<AppState>>,
    deep: Option<bool>,
) -> Result<SyncStatus, String> {
    // deep = 逐个发件箱解密扫描，只在设置页打开时用；主界面的状态条走轻量模式
    let deep = deep.unwrap_or(false);
    let settings = state.settings.lock().map_err(|e| e.to_string())?.clone();
    let dir = current_sync_dir(&state);
    let is_icloud = crate::state::detect_icloud_dir()
        .map(|d| dir.starts_with(&d))
        .unwrap_or(false);

    let (
        enabled,
        last_pull_at,
        last_error,
        peers,
        pushed_clips,
        pushed_images,
        pulled_clips,
        pulled_images,
        pending_images,
        waiting_images,
        skipped_oversize,
    ) = {
        let rt = state.sync.lock().map_err(|e| e.to_string())?;
        let mut peers: Vec<PeerStatus> = rt.peers.values().cloned().collect();
        peers.sort_by(|a, b| a.name.cmp(&b.name));
        (
            rt.enabled,
            rt.last_pull_at,
            rt.last_error.clone(),
            peers,
            rt.last_push_clips,
            rt.last_push_images,
            rt.pulled_clips,
            rt.pulled_images,
            rt.pending_images,
            rt.waiting_images,
            rt.skipped_oversize,
        )
    };

    // 扫一遍共享目录：每个发件箱能不能解开、里面有多少条。
    // 这是区分「没配对成功」和「配对成功但没东西可发」的唯一可靠办法。
    // deep 只在设置页打开时传 true——它要把每个发件箱都解密一遍，不适合每 3 秒轮询。
    let mut peer_files: Vec<PeerFile> = Vec::new();
    let mut own_clips = pushed_clips;
    let devices_dir = dir.join("JCopy").join("devices");
    let shared_images = dir.join("JCopy").join("images");
    if deep {
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
            let updated_at = meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);

            let is_self = id == settings.device_id;
            let mut decryptable = false;
            let mut clips = 0usize;
            let mut images = 0usize;
            let mut missing_images = 0usize;
            let mut error = None;

            match fs::read(&path) {
                Ok(blob) => match protocol::open_outbox(&id, &blob) {
                    Some(outbox) => {
                        decryptable = true;
                        clips = outbox.clips.len();
                        // 图片条目只带文件名，原图在 <目录>/JCopy/images 下。
                        // 数一下有几个原图还没到位——这能区分「配对没成功」和「云盘还在传」
                        for clip in outbox.clips.iter().filter(|c| c.kind == "image") {
                            images += 1;
                            let present = clip
                                .image_ref
                                .as_deref()
                                .map(|r| {
                                    std::path::Path::new(r)
                                        .file_name()
                                        .and_then(|s| s.to_str())
                                        .map(|name| shared_images.join(name).is_file())
                                        .unwrap_or(false)
                                })
                                .unwrap_or(false);
                            if !present {
                                missing_images += 1;
                            }
                        }
                    }
                    None if is_self => error = Some("本机发件箱解不开（文件可能损坏）".into()),
                    None => error = Some("读不了（文件损坏或来自旧版本）".into()),
                },
                Err(err) => error = Some(format!("读取失败：{err}")),
            }

            peer_files.push(PeerFile {
                id,
                updated_at,
                is_self,
                decryptable,
                clips,
                images,
                missing_images,
                error,
            });
        }
        }
    }
    peer_files.sort_by(|a, b| a.id.cmp(&b.id));
    // deep 扫描成功时以自己的发件箱为准；否则沿用最近一次 push 的运行时计数
    if let Some(file) = peer_files.iter().find(|f| f.is_self) {
        own_clips = file.clips;
    }

    let shared_image_count = count_shared_images(&shared_images, &settings.device_id);

    Ok(SyncStatus {
        enabled,
        dir: dir.display().to_string(),
        dir_exists: dir.is_dir(),
        is_icloud,
        device_name: settings.device_name,
        peers,
        last_pull_at,
        last_error,
        own_clips,
        peer_files,
        pushed_clips,
        pushed_images,
        pulled_clips,
        pulled_images,
        pending_images,
        waiting_images,
        skipped_oversize,
        shared_images: shared_image_count,
        dir_free_mb: free_mb(&dir),
    })
}

/// 共享目录里属于本机的图片文件数——就是"程序到底放了几张图到云盘目录"，
/// 用户可以直接拿这个数和另一台的文件夹对比，一眼看出是没放还是没传。
fn count_shared_images(dir: &Path, own_id: &str) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { return false };
            name.starts_with(own_id) && name.ends_with(".png")
        })
        .count()
}

/// 目录所在卷的剩余空间（MB）。拿不到时返回一个很大的值，界面就不报警。
fn free_mb(dir: &Path) -> u64 {
    #[cfg(unix)]
    {
        let Ok(out) = std::process::Command::new("df").arg("-m").arg(dir).output() else {
            return u64::MAX;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        // 第二行：Filesystem 1M-blocks Used Available Capacity Mounted
        if let Some(line) = text.lines().nth(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if let Some(avail) = cols.get(3).and_then(|v| v.parse::<u64>().ok()) {
                return avail;
            }
        }
        u64::MAX
    }

    #[cfg(target_os = "windows")]
    {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let mut free: u64 = 0;
        let wide = HSTRING::from(dir.display().to_string());
        let ok = unsafe {
            GetDiskFreeSpaceExW(&wide, Some(&mut free), None, None).is_ok()
        };
        if ok {
            free / (1024 * 1024)
        } else {
            u64::MAX
        }
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        let _ = dir;
        u64::MAX
    }
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

/// 「关于」页要展示的信息。版本取 Cargo 的，和 package.json / tauri.conf.json 同步升。
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub platform: String,
    pub arch: String,
    pub tauri: String,
    pub config_dir: String,
    pub sync_dir: String,
}

#[tauri::command]
pub fn app_info(state: State<'_, Arc<AppState>>) -> Result<AppInfo, String> {
    let platform = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Linux"
    };
    // Rust 的 ARCH 是 aarch64 / x86_64，界面上写 arm64 / x64 更好认
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    let sync_dir = state
        .sync
        .lock()
        .ok()
        .and_then(|rt| rt.dir.clone())
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: platform.to_string(),
        arch: arch.to_string(),
        tauri: tauri::VERSION.to_string(),
        config_dir: state.config_dir.display().to_string(),
        sync_dir,
    })
}

/// 当前是否已设置开机自启。
#[tauri::command]
pub fn autostart_enabled(app: AppHandle) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

/// 开关开机自启，返回设置后的真实状态（以系统为准，不是我们"以为"的值）。
#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if enabled {
        al.enable().map_err(|e| e.to_string())?;
    } else {
        al.disable().map_err(|e| e.to_string())?;
    }
    al.is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn sync_now(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    let inner = state.inner().clone();
    engine::sync_once(&app, &inner)
}

