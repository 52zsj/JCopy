use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

use crate::db;
use crate::model::Settings;
use crate::sync::crypto::Key;

/// 一台已配对设备在最近一次扫描中的状态。
#[derive(Serialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct PeerStatus {
    pub id: String,
    pub name: String,
    pub platform: String,
    /// 对端发件箱的更新时间（毫秒）
    pub updated_at: i64,
    /// 我们在本轮从该设备实际接收到的条数
    pub received: usize,
    pub online: bool,
}

#[derive(Default)]
pub struct SyncRuntime {
    pub enabled: bool,
    pub key: Option<Key>,
    pub dir: Option<PathBuf>,
    /// 上次写入的**明文**摘要，用于避免无变化的重复写盘
    /// （密文每次 nonce 不同，拿密文做摘要等于每轮都判定为"有变化"）
    pub last_push_hash: String,
    pub last_push_at: i64,
    pub last_pull_at: i64,
    /// 上次清理共享目录过期图片的时间，避免每轮都去扫目录
    pub last_prune_at: i64,
    /// 最近一次发件箱里实际写了多少条 / 其中几张图
    pub last_push_clips: usize,
    pub last_push_images: usize,
    /// 本次启动以来累计从对端收下多少条 / 其中几张图
    pub pulled_clips: usize,
    pub pulled_images: usize,
    /// 本机发件箱里有图片条目、但原图还没落进共享目录的张数（正常应为 0）
    pub pending_images: usize,
    /// 共享目录里有几个发件箱是本机解不开的（含旧配对残留）。
    /// 大于 0 说明"有设备的数据在这儿，但我们读不了"——通常是密钥不一致
    pub mismatched_peers: usize,
    pub last_error: Option<String>,
    pub peers: HashMap<String, PeerStatus>,
}

pub struct AppState {
    pub db: Mutex<Connection>,
    pub settings: Mutex<Settings>,
    pub paused: AtomicBool,
    /// 由 JCopy 自己写回剪贴板的内容，监听线程需跳过，避免自我回环。
    pub ignore_hash: Mutex<Option<String>>,
    pub config_dir: PathBuf,
    /// 图片条目落盘的位置（原图 + .t.png 缩略图）
    pub images_dir: PathBuf,
    pub sync: Mutex<SyncRuntime>,
    pub lan: Mutex<LanRuntime>,
}

/// 局域网直连的运行时状态：靠 mDNS 发现同网段的对端。
#[derive(Default)]
pub struct LanRuntime {
    pub peers: HashMap<String, SocketAddr>,
    pub port: u16,
    /// 服务发现用的守护进程，必须常驻，drop 掉广播就停了
    pub daemon: Option<mdns_sd::ServiceDaemon>,
}

pub fn init(app: &AppHandle) -> Result<AppState, Box<dyn std::error::Error>> {
    let config_dir = app.path().app_config_dir()?;
    fs::create_dir_all(&config_dir)?;

    let mut settings = load_settings(&config_dir);
    if settings.device_id.is_empty() {
        settings.device_id = db::new_id();
    }
    if settings.device_name.is_empty() {
        settings.device_name = default_device_name();
    }
    save_settings(&config_dir, &settings);

    let data_dir = app.path().app_data_dir()?;
    fs::create_dir_all(&data_dir)?;
    let images_dir = data_dir.join("images");
    fs::create_dir_all(&images_dir)?;
    let conn = db::open(&data_dir.join("jcopy.db"))?;

    let sync = SyncRuntime {
        enabled: settings.sync_enabled,
        key: settings.sync_key.as_deref().and_then(decode_key),
        dir: Some(settings.sync_dir.clone().map(PathBuf::from).unwrap_or_else(default_sync_dir)),
        ..Default::default()
    };

    Ok(AppState {
        db: Mutex::new(conn),
        settings: Mutex::new(settings),
        paused: AtomicBool::new(false),
        ignore_hash: Mutex::new(None),
        config_dir,
        images_dir,
        sync: Mutex::new(sync),
        lan: Mutex::new(LanRuntime::default()),
    })
}

pub fn load_settings(dir: &PathBuf) -> Settings {
    let path = dir.join("settings.json");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(dir: &PathBuf, settings: &Settings) {
    if let Ok(json) = serde_json::to_string_pretty(settings) {
        let path = dir.join("settings.json");
        let _ = fs::write(&path, json);
        // 里面含有同步主密钥，收紧到仅本人可读
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }
    }
}

/// 默认同步目录：优先 iCloud Drive，找不到就退到用户主目录下的一个可见文件夹。
pub fn default_sync_dir() -> PathBuf {
    if let Some(dir) = detect_icloud_dir() {
        return dir;
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("JCopy")
}

pub fn detect_icloud_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    let home = PathBuf::from(home);

    let candidates = [
        // macOS iCloud Drive 容器
        home.join("Library/Mobile Documents/com~apple~CloudDocs"),
        // Windows iCloud for Windows（新版无空格，旧版有空格）
        home.join("iCloudDrive"),
        home.join("iCloud Drive"),
    ];

    candidates.into_iter().find(|p| p.is_dir())
}

fn decode_key(b64: &str) -> Option<Key> {
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

fn default_device_name() -> String {
    let raw = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let fallback = if cfg!(target_os = "macos") { "Mac" } else { "PC" }.to_string();
    let name = raw.unwrap_or(fallback);

    // macOS 主机名常带 .local 后缀，展示时去掉
    name.strip_suffix(".local").unwrap_or(&name).to_string()
}
