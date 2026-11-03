//! 同步用到的密码学原语。
//!
//! 0.9.8 起不再有配对/密钥协商：两台设备登录同一个云盘账号，
//! 共享目录就是信任边界。密钥由程序内部固定派生，用户不需要做任何事。
//! 加密仍然保留——剪贴板里常有密码、token 这类东西，不该以明文躺在云盘上。

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use hkdf::Hkdf;
use sha2::Sha256;

pub type Key = [u8; 32];

const NONCE_LEN: usize = 12;

/// 固定主密钥。所有 JCopy 实例派生出同一把，因此两台设备能直接互读对方的发件箱，
/// 不需要任何配对步骤。
pub fn master_key() -> Key {
    let hk = Hkdf::<Sha256>::new(Some(b"jcopy-master-salt-v1"), b"jcopy-shared-master-v1");
    let mut out = [0u8; 32];
    hk.expand(b"master", &mut out)
        .expect("HKDF 输出长度固定，不会失败");
    out
}

pub fn device_file_key(master: &Key, device_id: &str) -> Key {
    let hk = Hkdf::<Sha256>::new(Some(b"jcopy-outbox-v1"), master);
    let mut out = [0u8; 32];
    hk.expand(device_id.as_bytes(), &mut out)
        .expect("HKDF 输出长度固定，不会失败");
    out
}

/// 输出 nonce || ciphertext
pub fn encrypt(key: &Key, plain: &[u8]) -> Option<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher.encrypt(&nonce, plain).ok()?;
    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(nonce.as_slice());
    out.extend_from_slice(&ciphertext);
    Some(out)
}

pub fn decrypt(key: &Key, blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() <= NONCE_LEN {
        return None;
    }
    let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    cipher.decrypt(Nonce::from_slice(nonce), ciphertext).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = master_key();
        let plain = b"hello jcopy";
        let blob = encrypt(&key, plain).unwrap();
        assert_ne!(&blob[12..], plain.as_slice(), "密文不应等于明文");
        assert_eq!(decrypt(&key, &blob).unwrap(), plain);
    }

    #[test]
    fn wrong_key_cannot_decrypt() {
        let blob = encrypt(&master_key(), b"secret").unwrap();
        // 换一把派生密钥就解不开
        let other = device_file_key(&master_key(), "other-device");
        assert!(decrypt(&other, &blob).is_none());
    }

    #[test]
    fn master_key_is_stable() {
        // 两台设备必须算出同一把，否则互相读不了
        assert_eq!(master_key(), master_key());
    }

    #[test]
    fn device_keys_are_isolated() {
        let master = master_key();
        assert_ne!(
            device_file_key(&master, "dev-a"),
            device_file_key(&master, "dev-b")
        );
    }
}
