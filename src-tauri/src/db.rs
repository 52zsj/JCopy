use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;

use crate::model::{preview_of, Clip};

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS clips (
            id           TEXT PRIMARY KEY,
            kind         TEXT NOT NULL,
            content      TEXT NOT NULL,
            hash         TEXT NOT NULL,
            preview      TEXT NOT NULL,
            source_app   TEXT,
            device       TEXT NOT NULL DEFAULT '本机',
            created_at   INTEGER NOT NULL,
            last_used_at INTEGER NOT NULL,
            pinned       INTEGER NOT NULL DEFAULT 0,
            bytes        INTEGER NOT NULL DEFAULT 0
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_clips_hash ON clips(hash);
        CREATE INDEX IF NOT EXISTS idx_clips_created ON clips(created_at DESC);",
    )?;
    Ok(conn)
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn new_id() -> String {
    format!("{:016x}", rand::random::<u64>())
}

pub fn hash_text(s: &str) -> String {
    hash_bytes(s.as_bytes())
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(&h.finalize()[..16])
}

pub enum InsertOutcome {
    Created(Clip),
    /// 已存在相同内容，只把它顶到最前
    Touched(String),
}

/// 命中已有 hash 时只更新排序时间，返回 Touched（让前端把该条移到列表首位）。
pub fn insert_or_touch(
    conn: &Connection,
    text: &str,
    source_app: Option<&str>,
    device: &str,
    hash: &str,
) -> rusqlite::Result<InsertOutcome> {
    let now = now_ms();
    let existing: Option<String> = conn
        .query_row("SELECT id FROM clips WHERE hash = ?1", [hash], |r| r.get(0))
        .optional()?;

    if let Some(id) = existing {
        conn.execute(
            "UPDATE clips SET created_at = ?1, last_used_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        return Ok(InsertOutcome::Touched(id));
    }

    let id = new_id();
    let preview = preview_of(text);
    let clip = Clip {
        id: id.clone(),
        kind: "text".into(),
        content: text.to_string(),
        preview: preview.clone(),
        hash: hash.to_string(),
        source_app: source_app.map(|s| s.to_string()),
        device: device.to_string(),
        created_at: now,
        last_used_at: now,
        pinned: false,
        bytes: text.len() as i64,
    };
    conn.execute(
        "INSERT INTO clips (id, kind, content, hash, preview, source_app, device, created_at, last_used_at, pinned, bytes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10)",
        params![
            clip.id,
            clip.kind,
            clip.content,
            clip.hash,
            preview,
            clip.source_app,
            clip.device,
            clip.created_at,
            clip.last_used_at,
            clip.bytes
        ],
    )?;
    Ok(InsertOutcome::Created(clip))
}

/// 同步去重用：只看哈希是否已存在，避免命中时被"顶到最前"。
pub fn has_hash(conn: &Connection, hash: &str) -> bool {
    conn.query_row("SELECT 1 FROM clips WHERE hash = ?1", [hash], |_| Ok(()))
        .is_ok()
}

/// 同步进来的条目：保留对端原始时间；已存在时完全不碰（不置顶、不改时间），
/// 否则每轮扫描都会把同一条顶上来。
pub fn insert_remote(
    conn: &Connection,
    text: &str,
    source_app: Option<&str>,
    device: &str,
    hash: &str,
    created_at: i64,
) -> rusqlite::Result<Option<Clip>> {
    if has_hash(conn, hash) {
        return Ok(None);
    }
    let id = new_id();
    let now = now_ms();
    let preview = preview_of(text);
    conn.execute(
        "INSERT INTO clips (id, kind, content, hash, preview, source_app, device, created_at, last_used_at, pinned, bytes)
         VALUES (?1, 'text', ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)",
        params![
            &id,
            text,
            hash,
            preview,
            source_app,
            device,
            created_at,
            now,
            text.len() as i64
        ],
    )?;
    Ok(Some(Clip {
        id,
        kind: "text".into(),
        content: text.to_string(),
        preview,
        hash: hash.to_string(),
        source_app: source_app.map(str::to_string),
        device: device.to_string(),
        created_at,
        last_used_at: now,
        pinned: false,
        bytes: text.len() as i64,
    }))
}

/// 图片条目：content 存原图绝对路径，preview 存 "1920 × 1080"
pub fn insert_image(
    conn: &Connection,
    path: &str,
    dims: &str,
    hash: &str,
    bytes: i64,
    source_app: Option<&str>,
    device: &str,
) -> rusqlite::Result<Option<Clip>> {
    if has_hash(conn, hash) {
        return Ok(None);
    }
    let id = new_id();
    let now = now_ms();
    conn.execute(
        "INSERT INTO clips (id, kind, content, hash, preview, source_app, device, created_at, last_used_at, pinned, bytes)
         VALUES (?1, 'image', ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)",
        params![&id, path, hash, dims, source_app, device, now, now, bytes],
    )?;
    Ok(Some(Clip {
        id,
        kind: "image".into(),
        content: path.to_string(),
        preview: dims.to_string(),
        hash: hash.to_string(),
        source_app: source_app.map(str::to_string),
        device: device.to_string(),
        created_at: now,
        last_used_at: now,
        pinned: false,
        bytes,
    }))
}

pub fn list(conn: &Connection, limit: i64) -> rusqlite::Result<Vec<Clip>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, content, preview, hash, source_app, device, created_at, last_used_at, pinned, bytes
         FROM clips
         ORDER BY pinned DESC, created_at DESC
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, Option<String>>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, i64>(7)?,
            r.get::<_, i64>(8)?,
            r.get::<_, i64>(9)?,
            r.get::<_, i64>(10)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, kind, content, preview, hash, source_app, device, created_at, last_used_at, pinned, bytes) =
            row?;
        out.push(Clip {
            id,
            kind,
            content,
            preview,
            hash,
            source_app,
            device,
            created_at,
            last_used_at,
            pinned: pinned != 0,
            bytes,
        });
    }
    Ok(out)
}

pub fn get(conn: &Connection, id: &str) -> rusqlite::Result<Option<Clip>> {
    conn.query_row(
        "SELECT id, kind, content, preview, hash, source_app, device, created_at, last_used_at, pinned, bytes
         FROM clips WHERE id = ?1",
        [id],
        |r| {
            Ok(Clip {
                id: r.get(0)?,
                kind: r.get(1)?,
                content: r.get(2)?,
                preview: r.get(3)?,
                hash: r.get(4)?,
                source_app: r.get(5)?,
                device: r.get(6)?,
                created_at: r.get(7)?,
                last_used_at: r.get(8)?,
                pinned: r.get::<_, i64>(9)? != 0,
                bytes: r.get(10)?,
            })
        },
    )
    .optional()
}

pub fn touch_used(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE clips SET last_used_at = ?1 WHERE id = ?2",
        params![now_ms(), id],
    )?;
    Ok(())
}

pub fn delete(conn: &Connection, id: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM clips WHERE id = ?1", [id])?;
    Ok(())
}

pub fn clear(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM clips", [])?;
    Ok(())
}

pub fn toggle_pin(conn: &Connection, id: &str) -> rusqlite::Result<bool> {
    conn.execute("UPDATE clips SET pinned = 1 - pinned WHERE id = ?1", [id])?;
    let pinned: i64 = conn
        .query_row("SELECT pinned FROM clips WHERE id = ?1", [id], |r| r.get(0))
        .unwrap_or(0);
    Ok(pinned != 0)
}

/// 双上限淘汰：先按条数，再按总体积。固定项不参与。
pub fn prune(conn: &Connection, max_clips: i64, max_bytes: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM clips WHERE pinned = 0 AND id NOT IN (
            SELECT id FROM clips WHERE pinned = 0 ORDER BY created_at DESC LIMIT ?1
         )",
        [max_clips],
    )?;

    let total: i64 = conn
        .query_row("SELECT COALESCE(SUM(bytes), 0) FROM clips", [], |r| r.get(0))
        .unwrap_or(0);

    if total > max_bytes {
        let mut over = total - max_bytes;
        let mut stmt = conn.prepare(
            "SELECT id, bytes FROM clips WHERE pinned = 0 ORDER BY created_at ASC",
        )?;
        let victims: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .filter_map(|r| r.ok())
            .collect();
        for (id, bytes) in victims {
            if over <= 0 {
                break;
            }
            conn.execute("DELETE FROM clips WHERE id = ?1", [&id])?;
            over -= bytes;
        }
    }
    Ok(())
}
