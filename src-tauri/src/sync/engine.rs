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

use super::protocol::{self, DeviceInfo, Outbox, SyncClip};
use crate::state::{AppState, PeerStatus};
use crate::db;

const TICK: Duration = Duration::from_secs(3);
const OUTBOX_LIMIT: usize = 200;
/// 超过这个体积的条目只留在本机，不参与同步
// 单条文本上限。原来 64KB，剪一段长日志/代码就超了，而且是静默不发
const MAX_SYNC_BYTES: i64 = 256 * 1024;
/// 图片单独一档上限：本机采集上限也是 12MB，两端对齐
const MAX_IMAGE_SYNC_BYTES: i64 = 12 * 1024 * 1024;
const MAX_PEER_FILES: usize = 16;
/// 共享图片保留天数。iCloud 空间不是无限的，只清自己写的那批
const SHARED_IMAGE_TTL_DAYS: u64 = 30;
/// 发件箱多久没更新就算"旧配对残留"（解不开也不再报警）
const STALE_OUTBOX_SECS: u64 = 10 * 60;
/// 共享目录清理间隔：十分钟扫一次，不必每轮都扫
const PRUNE_INTERVAL_MS: i64 = 10 * 60 * 1000;

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
    let (enabled, dir) = {
        let rt = state.sync.lock().map_err(|e| e.to_string())?;
        (rt.enabled, rt.dir.clone())
    };
    if !enabled {
        return Ok(());
    }
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

    push(state, &root, &device, &settings.image_sync)?;
    pull(app, state, &root, &settings.device_id)?;
    Ok(())
}

fn push(
    state: &Arc<AppState>,
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
    // 被过滤掉的东西必须计数：以前超体积、原图丢失都是静默跳过，
    // 用户只看到"另一台没收到"，无从判断是没发还是发不出去
    let mut skipped_oversize = 0usize;
    let mut missing_image_file = 0usize;

    let items: Vec<SyncClip> = clips
        .into_iter()
        .filter(|c| c.local)
        .filter(|c| {
            let too_big = match c.kind.as_str() {
                "image" => c.bytes > MAX_IMAGE_SYNC_BYTES,
                _ => c.bytes > MAX_SYNC_BYTES,
            };
            if too_big {
                skipped_oversize += 1;
                return false;
            }
            // 图片按策略过滤：off 不发，pinned 只发收藏过的
            match c.kind.as_str() {
                "image" => match image_mode {
                    "off" => false,
                    "all" => true,
                    _ => c.pinned,
                },
                _ => true,
            }
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
                    missing_image_file += 1;
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

    // 清理过期图片不必每轮都扫目录，十分钟一次足够
    if image_mode != "off" {
        let due = {
            let rt = state.sync.lock().map_err(|e| e.to_string())?;
            protocol::db_now_ms() - rt.last_prune_at > PRUNE_INTERVAL_MS
        };
        if due {
            prune_shared_images(&images_dir, &device.id);
            if let Ok(mut rt) = state.sync.lock() {
                rt.last_prune_at = protocol::db_now_ms();
            }
        }
    }

    // 摘要只算「条目内容」，不能算整份 Outbox：updated_at 每轮都变，
    // 更不能算密文——AES-GCM 每次用新 nonce，同一份内容密文也不同。
    // 以前取密文摘要，导致每 3 秒都判定"有变化"，文件被反复重写、iCloud 反复重传。
    let digest = digest_of(&serde_json::to_vec(&items).map_err(|e| e.to_string())?);

    {
        let rt = state.sync.lock().map_err(|e| e.to_string())?;
        if rt.last_push_hash == digest {
            return Ok(());
        }
    }

    // 移动进 outbox 之前先记数，供界面展示"这次到底发了什么"
    let clip_count = items.len();
    let image_count = items.iter().filter(|clip| clip.kind == "image").count();

    let outbox = Outbox {
        v: protocol::PROTOCOL_VERSION,
        device: device.clone(),
        updated_at: protocol::db_now_ms(),
        clips: items,
    };

    let sealed = protocol::seal_outbox(&device.id, &outbox).ok_or("发件箱加密失败")?;
    let path = root.join("devices").join(format!("{}.enc", device.id));
    write_atomic(&path, &sealed).map_err(|e| e.to_string())?;

    let mut rt = state.sync.lock().map_err(|e| e.to_string())?;
    rt.last_push_hash = digest;
    rt.last_push_at = protocol::db_now_ms();
    rt.last_push_clips = clip_count;
    rt.last_push_images = image_count;
    rt.pending_images = missing_image_file;
    rt.skipped_oversize = skipped_oversize;
    Ok(())
}

fn pull(
    app: &AppHandle,
    state: &Arc<AppState>,
    root: &Path,
    own_id: &str,
) -> Result<(), String> {
    let dir = root.join("devices");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(());
    };

    let mut found: Vec<PeerStatus> = Vec::new();
    let mut undecryptable: Vec<String> = Vec::new();
    // 本轮累计（写进 runtime 后按增量累加到总数）
    let mut total_received = 0usize;
    let mut total_images = 0usize;
    let mut mismatched = 0usize;
    let mut waiting_images = 0usize;

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
        let Some(outbox) = protocol::open_outbox(&peer_id, &blob) else {
            // 全部记下来供界面预警；但只有"新鲜的"才进错误文案，
            // 十分钟没动过的多半是上次配对的残留，不该每轮都弹红字
            mismatched += 1;
            if !is_stale(&path) {
                undecryptable.push(peer_id.clone());
            }
            continue;
        };

        let mut received = 0usize;
        let mut received_images = 0usize;
        for clip in &outbox.clips {
            // 对端说有这张图，但原图还没传过来（而本机也没有）→ 计入"正在等待"
            if clip.is_image() && !image_ready(state, root, clip) {
                waiting_images += 1;
                continue;
            }
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
                if saved.kind == "image" {
                    received_images += 1;
                }
                let _ = app.emit("clip-added", saved.view());
            }
        }
        total_received += received;
        total_images += received_images;

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
    rt.pulled_clips += total_received;
    rt.pulled_images += total_images;
    rt.mismatched_peers = mismatched;
    rt.waiting_images = waiting_images;
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
    // 0 字节多半是云盘的占位符还没真正落下来；超限则整个放弃
    if meta.len() == 0 || meta.len() > MAX_IMAGE_SYNC_BYTES as u64 {
        return Ok(None);
    }

    let local_id = db::new_id();
    let dest = state.images_dir.join(format!("{local_id}.png"));
    fs::copy(&source, &dest).map_err(|e| e.to_string())?;
    // 复制完再验一次大小：云盘按需下载时可能复制出 0 字节的空文件，
    // 那样一张"看起来同步了、点开是空白"的图比没有更糟
    let copied = fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    if copied == 0 {
        let _ = fs::remove_file(&dest);
        return Ok(None);
    }

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

/// 这张图的原图是否已经在共享目录里（或者本机早就有了，不需要再等）
fn image_ready(state: &Arc<AppState>, root: &Path, clip: &protocol::SyncClip) -> bool {
    if let Some(name) = clip.image_ref.as_deref() {
        let file = Path::new(name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if !file.is_empty() && root.join("images").join(file).is_file() {
            return true;
        }
    }
    state
        .db
        .lock()
        .map(|conn| db::has_hash(&conn, &clip.hash))
        .unwrap_or(false)
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
