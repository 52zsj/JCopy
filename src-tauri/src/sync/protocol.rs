//! 发件箱的线格式。
//!
//! 关键约束：每台设备只写自己的 `devices/<id>.enc`，只读别人的。
//! 这样从结构上就不存在"两台设备互相覆盖"的可能。
//!
//! 0.9.8 起没有配对/邀请文件：两台设备登录同一个云盘账号，
//! 共享目录就是通道，密钥由程序内部固定派生。

use serde::{Deserialize, Serialize};

use super::crypto;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub platform: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SyncClip {
    pub id: String,
    pub hash: String,
    /// "text" | "image"。老版本的发件箱没有这个字段，缺省按文本处理
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub text: String,
    /// 图片在共享目录 images/ 下的文件名（只存文件名，不存路径）。
    /// 图片本体不进发件箱，否则发件箱会随图片数量线性膨胀。
    #[serde(default)]
    pub image_ref: Option<String>,
    /// 图片尺寸，形如 "1920 × 1080"
    #[serde(default)]
    pub dims: Option<String>,
    pub source_app: Option<String>,
    pub created_at: i64,
    pub pinned: bool,
}

impl SyncClip {
    pub fn is_image(&self) -> bool {
        self.kind == "image"
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Outbox {
    pub v: u32,
    pub device: DeviceInfo,
    pub updated_at: i64,
    pub clips: Vec<SyncClip>,
}

pub fn current_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// 密钥按发件箱归属设备派生：谁写的文件只能用谁的 id 解开，
/// 避免误把别人的文件当成自己的。
fn file_key(device_id: &str) -> crypto::Key {
    crypto::device_file_key(&crypto::master_key(), device_id)
}

pub fn seal_outbox(device_id: &str, outbox: &Outbox) -> Option<Vec<u8>> {
    let json = serde_json::to_vec(outbox).ok()?;
    crypto::encrypt(&file_key(device_id), &json)
}

pub fn open_outbox(device_id: &str, blob: &[u8]) -> Option<Outbox> {
    let plain = crypto::decrypt(&file_key(device_id), blob)?;
    serde_json::from_slice(&plain).ok()
}

pub fn db_now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sample_outbox(device_id: &str, name: &str) -> Outbox {
        Outbox {
            v: PROTOCOL_VERSION,
            device: DeviceInfo {
                id: device_id.into(),
                name: name.into(),
                platform: "macos".into(),
            },
            updated_at: 1,
            clips: vec![SyncClip {
                id: "c1".into(),
                hash: "h1".into(),
                kind: "text".into(),
                text: "来自 A 的复制内容".into(),
                image_ref: None,
                dims: None,
                source_app: Some("com.apple.Safari".into()),
                created_at: 2,
                pinned: false,
            }],
        }
    }

    #[test]
    fn outbox_roundtrip() {
        let sealed = seal_outbox("a", &sample_outbox("a", "MacBook")).unwrap();
        let back = open_outbox("a", &sealed).unwrap();
        assert_eq!(back.clips.len(), 1);
        assert_eq!(back.clips[0].text, "来自 A 的复制内容");
    }

    #[test]
    fn outbox_rejects_foreign_device_key() {
        let sealed = seal_outbox("a", &sample_outbox("a", "MacBook")).unwrap();
        // 派生时绑定了 device_id，换 id 就解不开
        assert!(open_outbox("b", &sealed).is_none());
    }

    /// 两台设备经由共享目录直接互通，没有任何配对步骤。
    #[test]
    fn two_devices_exchange_through_shared_dir() {
        let root = std::env::temp_dir().join(format!("jcopy-test-{:016x}", rand::random::<u64>()));
        let devices = root.join("JCopy/devices");
        fs::create_dir_all(&devices).unwrap();

        // A 写自己的发件箱
        let sealed = seal_outbox("a", &sample_outbox("a", "MacBook")).unwrap();
        fs::write(devices.join("a.enc"), &sealed).unwrap();

        // B 直接读，不需要先做任何事
        let seen = open_outbox("a", &fs::read(devices.join("a.enc")).unwrap()).unwrap();
        assert_eq!(seen.clips.len(), 1);
        assert_eq!(seen.clips[0].text, "来自 A 的复制内容");
        assert_eq!(seen.device.name, "MacBook");

        // 换个 id 的文件就读不了
        assert!(open_outbox("zzz", &sealed).is_none());

        let _ = fs::remove_dir_all(&root);
    }
}
