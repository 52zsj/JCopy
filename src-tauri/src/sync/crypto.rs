//! 同步用到的密码学原语。
//!
//! 设计：一个主密钥（配对时在两台设备之间交换），每台设备的发件箱用
//! HKDF 派生出独立密钥加密。服务端（这里是 iCloud Drive）只见到密文。

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::{Algorithm, Argon2, Params, Version};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::Sha256;

pub type Key = [u8; 32];

const NONCE_LEN: usize = 12;

pub fn random_key() -> Key {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

pub fn random_salt() -> [u8; 16] {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    salt
}

/// 6 位配对码：3 位太弱，8 位输入成本高，6 位配合 Argon2id 是合适的折中。
pub fn random_pair_code() -> String {
    let n: u32 = rand::random::<u32>() % 1_000_000;
    format!("{n:06}")
}

pub fn device_file_key(master: &Key, device_id: &str) -> Key {
    let hk = Hkdf::<Sha256>::new(Some(b"jcopy-outbox-v1"), master);
    let mut out = [0u8; 32];
    hk.expand(device_id.as_bytes(), &mut out)
        .expect("HKDF 输出长度固定，不会失败");
    out
}

/// 主密钥指纹：给人眼比对用的 8 位十六进制。
/// 两台机器显示同一个指纹，才可能互相解开对方的发件箱。
pub fn fingerprint(master: &Key) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"jcopy-key-fp-v1");
    hasher.update(master);
    hex::encode(&hasher.finalize()[..4])
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

/// 局域网直连的会话密钥：换个 salt 做域分离，和云端的发件箱密钥互不影响。
pub fn decrypt(key: &Key, blob: &[u8]) -> Option<Vec<u8>> {
    if blob.len() <= NONCE_LEN {
        return None;
    }
    let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    cipher.decrypt(Nonce::from_slice(nonce), ciphertext).ok()
}

/// 用配对码派生邀请文件的解密密钥。
/// 32MB / 3 轮，单次约 80–120ms：用户无感，但把 100 万次的暴力枚举拉到 20 小时以上。
pub fn code_key(code: &str, salt: &[u8]) -> Option<Key> {
    let params = Params::new(32 * 1024, 3, 1, Some(32)).ok()?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon
        .hash_password_into(code.as_bytes(), salt, &mut out)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = random_key();
        let plain = b"hello jcopy";
        let blob = encrypt(&key, plain).unwrap();
        assert_ne!(&blob[12..], plain.as_slice(), "密文不应等于明文");
        assert_eq!(decrypt(&key, &blob).unwrap(), plain);
    }

    #[test]
    fn wrong_key_cannot_decrypt() {
        let blob = encrypt(&random_key(), b"secret").unwrap();
        assert!(decrypt(&random_key(), &blob).is_none());
    }

    #[test]
    fn device_keys_are_isolated() {
        let master = random_key();
        assert_ne!(
            device_file_key(&master, "dev-a"),
            device_file_key(&master, "dev-b")
        );
    }

    #[test]
    fn pair_code_is_always_six_digits() {
        for _ in 0..50 {
            let code = random_pair_code();
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn code_key_depends_on_both_inputs() {
        let s1 = random_salt();
        let s2 = random_salt();
        assert_ne!(code_key("123456", &s1).unwrap(), code_key("123456", &s2).unwrap());
        assert_ne!(code_key("123456", &s1).unwrap(), code_key("654321", &s1).unwrap());
        assert_eq!(code_key("123456", &s1).unwrap(), code_key("123456", &s1).unwrap());
    }
}
