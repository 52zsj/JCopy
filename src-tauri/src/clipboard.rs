use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::state::AppState;
use crate::{db, platform};

const POLL_INTERVAL: Duration = Duration::from_millis(350);

/// 常驻监听线程：轮询系统剪贴板，变化时入库并通知前端。
/// 各平台统一用内容哈希去重，避免依赖各自的事件模型差异。
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

            let text = match clipboard.get_text() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if text.trim().is_empty() {
                continue;
            }

            let hash = db::hash_text(&text);
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

            let inserted = {
                let conn = state.db.lock().unwrap();
                match db::insert_or_touch(&conn, &text, source.as_deref(), &device, &hash) {
                    Ok(outcome) => {
                        let _ = db::prune(&conn, max_clips, max_bytes);
                        Some(outcome)
                    }
                    Err(e) => {
                        eprintln!("[jcopy] 写入历史失败: {e}");
                        None
                    }
                }
            };

            match inserted {
                Some(db::InsertOutcome::Created(clip)) => {
                    let _ = app.emit("clip-added", clip.view());
                }
                Some(db::InsertOutcome::Touched(id)) => {
                    let _ = app.emit("clip-touched", id);
                }
                None => {}
            }
        }
    });
}
