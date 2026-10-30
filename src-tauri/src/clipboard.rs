use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::state::AppState;
use crate::{db, platform};

const POLL_INTERVAL: Duration = Duration::from_millis(350);
/// 单张图片上限：整屏 Retina 截图约 6–10MB，再大就不往历史和同步里塞了
const MAX_IMAGE_BYTES: usize = 12 * 1024 * 1024;
/// 卡片显示的缩略图宽度，够撑满 220pt 的卡片 @2x
const THUMB_WIDTH: u32 = 640;

/// 常驻监听线程：轮询系统剪贴板，变化时入库并通知前端。
/// 文本与图片共用一套哈希去重；自己写回的内容靠 ignore_hash 跳过。
pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    thread::spawn(move || {
        let mut clipboard = match arboard::Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[jcopy] 剪贴板初始化失败: {e}");
                return;
            }
        };

        let device = state.settings.lock().unwrap().device_name.clone();
        let mut last_hash = String::new();

        loop {
            thread::sleep(POLL_INTERVAL);

            if state.paused.load(Ordering::Relaxed) {
                continue;
            }

            let text = clipboard.get_text().ok().filter(|t| !t.trim().is_empty());
            let image = if text.is_none() {
                clipboard.get_image().ok()
            } else {
                None
            };
            if text.is_none() && image.is_none() {
                continue;
            }

            let hash = match (&text, &image) {
                (Some(t), _) => db::hash_text(t),
                (_, Some(img)) => db::hash_bytes(&img.bytes),
                _ => continue,
            };

            if hash == last_hash {
                continue;
            }
            last_hash.clone_from(&hash);

            // 自己写回剪贴板的内容不计入历史
            {
                let mut ignore = state.ignore_hash.lock().unwrap();
                if ignore.as_deref() == Some(hash.as_str()) {
                    *ignore = None;
                    continue;
                }
            }

            let source = platform::frontmost_app();
            if let Some(src) = source.as_deref() {
                let blocked = {
                    let s = state.settings.lock().unwrap();
                    s.blacklist.iter().any(|b| b == src)
                };
                if blocked {
                    continue;
                }
            }

            let (max_clips, max_bytes) = {
                let s = state.settings.lock().unwrap();
                (s.max_clips, s.max_bytes)
            };

            let inserted = if let Some(t) = &text {
                let conn = state.db.lock().unwrap();
                match db::insert_or_touch(&conn, t, source.as_deref(), &device, &hash) {
                    Ok(outcome) => Some(outcome),
                    Err(e) => {
                        eprintln!("[jcopy] 写入历史失败: {e}");
                        None
                    }
                }
            } else if let Some(img) = &image {
                // 图片先查重再落盘，避免同一张图反复写文件
                let exists = {
                    let conn = state.db.lock().unwrap();
                    db::has_hash(&conn, &hash)
                };
                if exists {
                    let conn = state.db.lock().unwrap();
                    conn.query_row("SELECT id FROM clips WHERE hash = ?1", [&hash], |r| {
                        r.get::<_, String>(0)
                    })
                    .ok()
                    .map(db::InsertOutcome::Touched)
                } else {
                    save_image(&state.images_dir, img).and_then(|(path, dims, bytes)| {
                        let conn = state.db.lock().unwrap();
                        db::insert_image(&conn, &path, &dims, &hash, bytes, source.as_deref(), &device)
                            .ok()
                            .flatten()
                            .map(db::InsertOutcome::Created)
                    })
                }
            } else {
                None
            };

            if inserted.is_some() {
                let conn = state.db.lock().unwrap();
                let _ = db::prune(&conn, max_clips, max_bytes);
            }

            match inserted {
                Some(db::InsertOutcome::Created(clip)) => {
                    let _ = app.emit("clip-added", clip.view());
                    // 只有文本走局域网直推；图片体积大，交给 iCloud 慢慢搬
                    if clip.kind == "text" {
                        let payload = crate::sync::SyncClip {
                            id: clip.id.clone(),
                            hash: clip.hash.clone(),
                            text: clip.content.clone(),
                            source_app: clip.source_app.clone(),
                            created_at: clip.created_at,
                            pinned: clip.pinned,
                        };
                        crate::sync::lan::broadcast_one(app.clone(), state.clone(), payload);
                    }
                }
                Some(db::InsertOutcome::Touched(id)) => {
                    let _ = app.emit("clip-touched", id);
                }
                None => {}
            }
        }
    });
}

/// 原图存 `<id>.png`，缩略图存 `<id>.t.png`（同目录，前端按路径后缀推出来）。
fn save_image(dir: &Path, img: &arboard::ImageData) -> Option<(String, String, i64)> {
    if img.bytes.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let id = db::new_id();
    let rgba = image::RgbaImage::from_raw(
        img.width as u32,
        img.height as u32,
        img.bytes.to_vec(),
    )?;

    let path = dir.join(format!("{id}.png"));
    rgba.save(&path).ok()?;

    let thumb_w = THUMB_WIDTH.min(rgba.width()).max(1);
    let ratio = thumb_w as f32 / rgba.width() as f32;
    let thumb_h = ((rgba.height() as f32) * ratio).round().max(1.0) as u32;
    let thumb = image::imageops::resize(&rgba, thumb_w, thumb_h, image::imageops::FilterType::Triangle);
    thumb.save(dir.join(format!("{id}.t.png"))).ok()?;

    let bytes = std::fs::metadata(&path).ok()?.len() as i64;
    Some((
        path.display().to_string(),
        format!("{} × {}", img.width, img.height),
        bytes,
    ))
}
