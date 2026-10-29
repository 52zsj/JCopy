use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

use crate::db;
use crate::model::Settings;

pub struct AppState {
    pub db: Mutex<Connection>,
    pub settings: Mutex<Settings>,
    pub paused: AtomicBool,
    /// 由 JCopy 自己写回剪贴板的内容，监听线程需跳过，避免自我回环。
    pub ignore_hash: Mutex<Option<String>>,
    pub config_dir: PathBuf,
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
    let conn = db::open(&data_dir.join("jcopy.db"))?;

    Ok(AppState {
        db: Mutex::new(conn),
        settings: Mutex::new(settings),
        paused: AtomicBool::new(false),
        ignore_hash: Mutex::new(None),
        config_dir,
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
        let _ = fs::write(dir.join("settings.json"), json);
    }
}

fn default_device_name() -> String {
    let name = std::process::Command::new("hostname")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    match name {
        Some(n) => n,
        None => if cfg!(target_os = "macos") { "Mac" } else { "PC" }.to_string(),
    }
}
