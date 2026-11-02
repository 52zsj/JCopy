//! 同步引擎：把本机条目写进自己的发件箱，并把对端发件箱里的新条目取回来。
//!
//! 数据流刻意做成"单向投递 + 拉取"：本机只写 `devices/<自己>.enc`，只读别人的文件。
//! 因此两台设备同时运行也不会互相覆盖——这一点是照搬 JShell 踩过的教训。

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime};
use tauri::{AppHandle, Emitter};

use super::crypto;
use super::protocol::{self, DeviceInfo, Outbox, SyncClip};
use crate::state::{AppState, PeerStatus};
use crate::db;

const TICK: Duration = Duration::from_secs(3);
const OUTBOX_LIMIT: usize = 200;
/// 超过这个体积的条目只留在本机，不参与同步
const MAX_SYNC_BYTES: i64 = 64 * 1024;
/// 图片单独一档上限：本机采集上限也是 12MB，两端对齐
const MAX_IMAGE_SYNC_BYTES: i64 = 12 * 1024 * 1024;
const MAX_PEER_FILES: usize = 16;
/// 共享图片保留天数。iCloud 空间不是无限的，只清自己写的那批
const SHARED_IMAGE_TTL_DAYS: u64 = 30;
/// 发件箱多久没更新就算"旧配对残留"（解不开也不再报警）
const STALE_OUTBOX_SECS: u64 = 10 * 60;

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

    push(state, &key, &root, &device, &settings.image_sync)?;
    pull(app, state, &key, &root, &settings.device_id)?;
    Ok(())
}

fn push(
    state: &Arc<AppState>,
    key: &crypto::Key,
    root: &Path,
    device: &DeviceInfo,
    image_mode: &str,
) -> Result<(), String> {
    let clips = {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        db::list(&conn, 400).unwrap_or_default()
    };

    let images_dir = root.join("images");
    if image_mode != "off" && !images_dir.is_dir() {
        fs::create_dir_all(&images_dir).map_err(|e| e.to_string())?;
    }

    // 只发本机产生的条目：从对端同步来的内容不再回传，天然切断回环。
    // 用 c.local 判断而不是比对设备名——改过设备名之后，历史条目的 device 还是旧名字，
    // 按名字比会把整段历史漏发（这正是"B 机器一直收不到东西"的一个原因）。
    let items: Vec<SyncClip> = clips
        .into_iter()
        .filter(|c| c.local)
        // 图片按策略过滤——默认只有收藏过的才出去，避免把 iCloud 塞满
        .filter(|c| match c.kind.as_str() {
            "image" => match image_mode {
                "off" => false,
                "all" => c.bytes <= MAX_IMAGE_SYNC_BYTES,
                _ => c.pinned && c.bytes <= MAX_IMAGE_SYNC_BYTES,
            },
            _ => c.bytes <= MAX_SYNC_BYTES,
        })
        .take(OUTBOX_LIMIT)
        .filter_map(|c| {
            if c.kind == "image" {
                // 图片本体写进共享目录，发件箱只带文件名。
                // 文件名用 deviceId 前缀，清理时只删自己的，不会误伤对端。
                let name = format!("{}-{}.png", device.id, c.id);
                let target = images_dir.join(&name);
                if !target.exists() && fs::copy(&c.content, &target).is_err() {
                    // 本地原图已被清理，这一条就不发了
                    return None;
                }
                Some(SyncClip {
                    id: c.id,
                    hash: c.hash,
                    kind: "image".into(),
                    text: String::new(),
                    image_ref: Some(name),
                    dims: Some(c.preview),
                    source_app: c.source_app,
                    created_at: c.created_at,
                    pinned: c.pinned,
                })
            } else {
                Some(SyncClip {
                    id: c.id,
                    hash: c.hash,
                    kind: "text".into(),
                    text: c.content,
                    image_ref: None,
                    dims: None,
                    source_app: c.source_app,
                    created_at: c.created_at,
                    pinned: c.pinned,
                })
            }
        })
        .collect();

    if image_mode != "off" {
        prune_shared_images(&images_dir, &device.id);
    }

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
    let mut undecryptable: Vec<String> = Vec::new();

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
        // 解不开基本只有一种原因：两台机器的主密钥不一致（各自生成过配对码）。
        // 以前这里静默 continue，界面看不出任何异常——现在记下来，让用户能看见。
        let Some(outbox) = protocol::open_outbox(key, &peer_id, &blob) else {
            // 但十分钟没动过的解不开文件属于上次配对的残留，不该一直报警
            if !is_stale(&path) {
                undecryptable.push(peer_id.clone());
            }
            continue;
        };

        let mut received = 0usize;
        for clip in &outbox.clips {
            let added = if clip.is_image() {
                pull_image(state, root, clip, &outbox.device.name)?
            } else if clip.text.trim().is_empty() {
                // 空文本多半是旧版本把图片路径当文本发过来的残渣，直接丢弃
                None
            } else {
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
    rt.last_error = if undecryptable.is_empty() {
        None
    } else {
        eprintln!(
            "[jcopy] 有 {} 个对端发件箱解不开：{}",
            undecryptable.len(),
            undecryptable.join(", ")
        );
        Some(format!(
            "有 {} 个对端发件箱解不开（本机与对方的主密钥不一致）：{}",
            undecryptable.len(),
            undecryptable.join("、")
        ))
    };
    rt.peers = found.into_iter().map(|p| (p.id.clone(), p)).collect();
    Ok(())
}

/// 拉取一条图片：从共享目录把原图拷回本机、本地重建缩略图、按 hash 去重入库。
/// 图片可能晚于发件箱到达 iCloud（大文件更慢），文件不在就跳过，下一轮再试。
fn pull_image(
    state: &Arc<AppState>,
    root: &Path,
    clip: &SyncClip,
    from_device: &str,
) -> Result<Option<crate::model::Clip>, String> {
    let Some(name) = clip.image_ref.as_deref() else {
        return Ok(None);
    };
    // 只取文件名，防止 image_ref 里塞 ../ 之类的路径穿越
    let file_name = Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    if file_name.is_empty() {
        return Ok(None);
    }

    {
        let conn = state.db.lock().map_err(|e| e.to_string())?;
        if db::has_hash(&conn, &clip.hash) {
            return Ok(None);
        }
    }

    let source = root.join("images").join(&file_name);
    let Ok(meta) = fs::metadata(&source) else {
        return Ok(None);
    };
    if meta.len() > MAX_IMAGE_SYNC_BYTES as u64 {
        return Ok(None);
    }

    let local_id = db::new_id();
    let dest = state.images_dir.join(format!("{local_id}.png"));
    fs::copy(&source, &dest).map_err(|e| e.to_string())?;

    let thumb = state.images_dir.join(format!("{local_id}.t.png"));
    if crate::clipboard::make_thumbnail(&dest, &thumb).is_none() {
        let _ = fs::remove_file(&dest);
        return Ok(None);
    }

    let dims = clip.dims.clone().unwrap_or_default();
    let conn = state.db.lock().map_err(|e| e.to_string())?;
    db::insert_image(
        &conn,
        &dest.display().to_string(),
        &dims,
        &clip.hash,
        meta.len() as i64,
        clip.source_app.as_deref(),
        from_device,
        false,
    )
    .map_err(|e| e.to_string())
}

/// 清理自己写进共享目录的过期图片。只认自己的前缀，别人的一张不动。
fn prune_shared_images(dir: &Path, own_id: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let Some(cutoff) = std::time::SystemTime::now()
        .checked_sub(Duration::from_secs(SHARED_IMAGE_TTL_DAYS * 24 * 3600))
    else {
        return;
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        if !name.starts_with(own_id) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified < cutoff {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// 文件超过十分钟没更新：多半是上一次配对的残留，不再当作"当前配对失败"
fn is_stale(path: &Path) -> bool {
    let Ok(meta) = fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    SystemTime::now()
        .duration_since(modified)
        .map(|age| age.as_secs() > STALE_OUTBOX_SECS)
        .unwrap_or(false)
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
