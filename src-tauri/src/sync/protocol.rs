//! 发件箱与配对邀请的线格式。
//!
//! 关键约束：每台设备只写自己的 `devices/<id>.enc`，只读别人的。
//! 这样从结构上就不存在"两台设备互相覆盖"的可能。

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};

use super::crypto::{self, Key};

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
    pub text: String,
    pub source_app: Option<String>,
    pub created_at: i64,
    pub pinned: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Outbox {
    pub v: u32,
    pub device: DeviceInfo,
    pub updated_at: i64,
    pub clips: Vec<SyncClip>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Invite {
    pub v: u32,
    pub invite_id: String,
    pub salt: String,
    /// nonce || ciphertext，再用配对码派生的密钥加密
    pub sealed: String,
    pub expires_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InvitePayload {
    /// 主密钥（base64）
    pub key: String,
    pub device_id: String,
    pub device_name: String,
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

pub fn seal_outbox(master: &Key, device_id: &str, outbox: &Outbox) -> Option<Vec<u8>> {
    let file_key = crypto::device_file_key(master, device_id);
    let json = serde_json::to_vec(outbox).ok()?;
    crypto::encrypt(&file_key, &json)
}

pub fn open_outbox(master: &Key, device_id: &str, blob: &[u8]) -> Option<Outbox> {
    let file_key = crypto::device_file_key(master, device_id);
    let plain = crypto::decrypt(&file_key, blob)?;
    serde_json::from_slice(&plain).ok()
}

pub fn seal_invite(code: &str, payload: &InvitePayload) -> Option<Invite> {
    let salt = crypto::random_salt();
    let key = crypto::code_key(code, &salt)?;
    let json = serde_json::to_vec(payload).ok()?;
    let sealed = crypto::encrypt(&key, &json)?;
    Some(Invite {
        v: PROTOCOL_VERSION,
        invite_id: format!("{:016x}", rand::random::<u64>()),
        salt: B64.encode(salt),
        sealed: B64.encode(sealed),
        expires_at: 0,
    })
}

pub fn open_invite(code: &str, invite: &Invite) -> Option<InvitePayload> {
    if invite.expires_at > 0 && db_now_ms() > invite.expires_at {
        return None;
    }
    let salt = B64.decode(&invite.salt).ok()?;
    let sealed = B64.decode(&invite.sealed).ok()?;
    let key = crypto::code_key(code, &salt)?;
    let plain = crypto::decrypt(&key, &sealed)?;
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
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use std::fs;

    fn encode_key(key: crypto::Key) -> String {
        B64.encode(key)
    }

    fn decode_key(s: &str) -> crypto::Key {
        let bytes = B64.decode(s).unwrap();
        let mut out = [0u8; 32];
        out.copy_from_slice(&bytes);
        out
    }

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
                text: "来自 A 的复制内容".into(),
                source_app: Some("com.apple.Safari".into()),
                created_at: 2,
                pinned: false,
            }],
        }
    }

    #[test]
    fn outbox_roundtrip() {
        let master = crypto::random_key();
        let sealed = seal_outbox(&master, "a", &sample_outbox("a", "MacBook")).unwrap();
        let back = open_outbox(&master, "a", &sealed).unwrap();
        assert_eq!(back.clips.len(), 1);
        assert_eq!(back.clips[0].text, "来自 A 的复制内容");
    }

    #[test]
    fn outbox_rejects_foreign_device_key() {
        let master = crypto::random_key();
        let sealed = seal_outbox(&master, "a", &sample_outbox("a", "MacBook")).unwrap();
        // 同一主密钥，但派生时绑定了 device_id，换 id 就解不开
        assert!(open_outbox(&master, "b", &sealed).is_none());
    }

    #[test]
    fn invite_roundtrip() {
        let payload = InvitePayload {
            key: encode_key(crypto::random_key()),
            device_id: "a".into(),
            device_name: "MacBook".into(),
        };
        let invite = seal_invite("123456", &payload).unwrap();
        let opened = open_invite("123456", &invite).unwrap();
        assert_eq!(opened.device_id, "a");
        assert_eq!(opened.device_name, "MacBook");
    }

    #[test]
    fn wrong_code_cannot_open_invite() {
        let payload = InvitePayload {
            key: encode_key(crypto::random_key()),
            device_id: "a".into(),
            device_name: "MacBook".into(),
        };
        let invite = seal_invite("123456", &payload).unwrap();
        assert!(open_invite("000000", &invite).is_none());
    }

    #[test]
    fn expired_invite_rejected() {
        let payload = InvitePayload {
            key: encode_key(crypto::random_key()),
            device_id: "a".into(),
            device_name: "MacBook".into(),
        };
        let mut invite = seal_invite("123456", &payload).unwrap();
        invite.expires_at = db_now_ms() - 1;
        assert!(open_invite("123456", &invite).is_none());
    }

    /// 模拟两台设备经由共享目录完成一次「配对 → 投递 → 取回」。
    #[test]
    fn two_devices_exchange_through_shared_dir() {
        let root = std::env::temp_dir().join(format!("jcopy-test-{:016x}", rand::random::<u64>()));
        let devices = root.join("JCopy/devices");
        let pair = root.join("JCopy/pair");
        fs::create_dir_all(&devices).unwrap();
        fs::create_dir_all(&pair).unwrap();

        // A：生成主密钥与邀请
        let master_a = crypto::random_key();
        let code = "424242";
        let payload = InvitePayload {
            key: encode_key(master_a),
            device_id: "a".into(),
            device_name: "MacBook".into(),
        };
        let mut invite = seal_invite(code, &payload).unwrap();
        invite.expires_at = db_now_ms() + 600_000;
        let invite_path = pair.join(format!("{}.invite", invite.invite_id));
        fs::write(&invite_path, serde_json::to_vec(&invite).unwrap()).unwrap();

        // B：用配对码解开邀请，拿到同一把主密钥
        let parsed: Invite = serde_json::from_slice(&fs::read(&invite_path).unwrap()).unwrap();
        let opened = open_invite(code, &parsed).unwrap();
        let master_b = decode_key(&opened.key);
        assert_eq!(master_b, master_a, "配对后两端主密钥必须一致");

        // A 写自己的发件箱
        let sealed = seal_outbox(&master_a, "a", &sample_outbox("a", "MacBook")).unwrap();
        fs::write(devices.join("a.enc"), &sealed).unwrap();

        // B 读取 A 的内容
        let seen = open_outbox(&master_b, "a", &fs::read(devices.join("a.enc")).unwrap()).unwrap();
        assert_eq!(seen.clips.len(), 1);
        assert_eq!(seen.clips[0].text, "来自 A 的复制内容");
        assert_eq!(seen.device.name, "MacBook");

        // 未配对的第三方读不出内容
        assert!(open_outbox(&crypto::random_key(), "a", &sealed).is_none());

        let _ = fs::remove_dir_all(&root);
    }
}
