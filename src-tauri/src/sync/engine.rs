//! 同步引擎：把本机条目写进自己的发件箱，并把对端发件箱里的新条目取回来。
//!
//! 数据流刻意做成"单向投递 + 拉取"：本机只写 `devices/<自己>.enc`，只读别人的文件。
//! 因此两台设备同时运行也不会互相覆盖——这一点是照搬 JShell 踩过的教训。

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use super::crypto;
use super::protocol::{self, DeviceInfo, Outbox, SyncClip};
use crate::state::{AppState, PeerStatus};
use crate::db;

const TICK: Duration = Duration::from_secs(3);
const OUTBOX_LIMIT: usize = 200;
/// 超过这个体积的条目只留在本机，不参与同步
const MAX_SYNC_BYTES: i64 = 64 * 1024;
const MAX_PEER_FILES: usize = 16;

pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    thread::spawn(move || loop {
        thread::sleep(TICK);
        if let Err(err) = tick(&app, &state) {
            if let Ok(mut rt) = state.sync.lock() {
                rt.last_error = Some(err);
            }
        }
    });
}

/// 供「立即同步」按钮调用
pub fn sync_once(app: &AppHandle, state: &Arc<AppState>) -> Result<(), String> {
    tick(app, state)
}

fn tick(app: &AppHandle, state: &Arc<AppState>) -> Result<(), String> {
    let (enabled, key, dir) = {
        let rt = state.sync.lock().map_err(|e| e.to_string())?;
        (rt.enabled, rt.key, rt.dir.clone())
    };
    if !enabled {
        return Ok(());
    }
    let Some(key) = key else { return Ok(()) };
    let Some(dir) = dir else { return Ok(()) };
    if !dir.is_dir() {
        return Err(format!("同步目录不可用：{}", dir.display()));
    }

    let root = dir.join("JCopy");
    fs::create_dir_all(root.join("devices")).map_err(|e| e.to_string())?;

    let settings = state.settings.lock().map_err(|e| e.to_string())?.clone();
    let device = DeviceInfo {
        id: settings.device_id.clone(),
        name: settings.device_name.clone(),
        platform: protocol::current_platform().to_string(),
    };

    push(state, &key, &root, &device)?;
    pull(app, state, &key, &root, &settings.device_id)?;
    Ok(())
}

fn push(
    state: &Arc<AppState>,
    key: &crypto::Key,
    root: &Path,
    device: &DeviceInfo,
) -> Result<(), String> {
    let clips = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::list(&conn, 400).unwrap_or_default()
    };

    // 只发本机产生的条目：从对端同步来的内容不再回传，天然切断回环
    let items: Vec<SyncClip> = clips
        .into_iter()
        .filter(|c| c.device == device.name && c.bytes <= MAX_SYNC_BYTES)
        .take(OUTBOX_LIMIT)
        .map(|c| SyncClip {
            id: c.id,
            hash: c.hash,
            text: c.content,
            source_app: c.source_app,
            created_at: c.created_at,
            pinned: c.pinned,
        })
        .collect();

    let outbox = Outbox {
        v: protocol::PROTOCOL_VERSION,
        device: device.clone(),
        updated_at: protocol::db_now_ms(),
        clips: items,
    };

    let sealed = protocol::seal_outbox(key, &device.id, &outbox).ok_or("发件箱加密失败")?;
    let digest = digest_of(&sealed);

    {
        let rt = state.sync.lock().map_err(|e| e.to_string())?;
        if rt.last_push_hash == digest {
            return Ok(());
        }
    }

    let path = root.join("devices").join(format!("{}.enc", device.id));
    write_atomic(&path, &sealed).map_err(|e| e.to_string())?;

    let mut rt = state.sync.lock().map_err(|e| e.to_string())?;
    rt.last_push_hash = digest;
    rt.last_push_at = protocol::db_now_ms();
    Ok(())
}

fn pull(
    app: &AppHandle,
    state: &Arc<AppState>,
    key: &crypto::Key,
    root: &Path,
    own_id: &str,
) -> Result<(), String> {
    let dir = root.join("devices");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(());
    };

    let mut found: Vec<PeerStatus> = Vec::new();

    for entry in entries.flatten().take(MAX_PEER_FILES) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("enc") {
            continue;
        }
        let Some(peer_id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_string) else {
            continue;
        };
        if peer_id == own_id {
            continue;
        }

        let Ok(blob) = fs::read(&path) else { continue };
        // 解不开说明不是同一配对组（或文件损坏），静默跳过，不打扰用户
        let Some(outbox) = protocol::open_outbox(key, &peer_id, &blob) else {
            continue;
        };

        let mut received = 0usize;
        for clip in &outbox.clips {
            let added = {
                let conn = state.db.lock().map_err(|e| e.to_string())?;
                match db::insert_remote(
                    &conn,
                    &clip.text,
                    clip.source_app.as_deref(),
                    &outbox.device.name,
                    &clip.hash,
                    clip.created_at,
                ) {
                    Ok(Some(saved)) => Some(saved),
                    _ => None,
                }
            };
            if let Some(saved) = added {
                received += 1;
                let _ = app.emit("clip-added", saved.view());
            }
        }

        found.push(PeerStatus {
            id: peer_id,
            name: outbox.device.name,
            platform: outbox.device.platform,
            updated_at: outbox.updated_at,
            received,
            online: true,
        });
    }

    let mut rt = state.sync.lock().map_err(|e| e.to_string())?;
    rt.last_pull_at = protocol::db_now_ms();
    rt.last_error = None;
    rt.peers = found.into_iter().map(|p| (p.id.clone(), p)).collect();
    Ok(())
}

/// 先写临时文件再改名：避免对端读到写了一半的密文
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("enc.tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

fn digest_of(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(&hasher.finalize()[..16])
}
