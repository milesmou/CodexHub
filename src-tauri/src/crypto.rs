//! 凭证加密：账号库里存的是完整 OAuth token，绝不能明文落盘。
//!
//! 方案：AES-256-GCM 加密整个账号库文件，主密钥交给 Windows 凭据管理器保管
//! （通过 keyring 写入，只有当前 Windows 用户可读）。

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Result};
use base64::Engine as _;
use rand::RngCore;

/// 凭据管理器里的服务名与条目名
const SERVICE: &str = "codex-helper";
const ENTRY: &str = "vault-master-key";

/// GCM nonce 固定 12 字节
const NONCE_LEN: usize = 12;

/// 取出主密钥；首次运行时随机生成并写入凭据管理器。
fn load_or_create_key() -> Result<[u8; 32]> {
    let entry = keyring::Entry::new(SERVICE, ENTRY)
        .map_err(|e| anyhow!("无法访问系统凭据管理器：{e}"))?;

    match entry.get_password() {
        Ok(encoded) => {
            let raw = base64::engine::general_purpose::STANDARD
                .decode(encoded.trim())
                .map_err(|e| anyhow!("主密钥解码失败：{e}"))?;
            if raw.len() != 32 {
                return Err(anyhow!("主密钥长度异常：{}", raw.len()));
            }
            let mut key = [0u8; 32];
            key.copy_from_slice(&raw);
            Ok(key)
        }
        // 第一次运行，生成一把新的
        Err(keyring::Error::NoEntry) => {
            let mut key = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut key);
            let encoded = base64::engine::general_purpose::STANDARD.encode(key);
            entry
                .set_password(&encoded)
                .map_err(|e| anyhow!("主密钥写入凭据管理器失败：{e}"))?;
            Ok(key)
        }
        Err(e) => Err(anyhow!("读取主密钥失败：{e}")),
    }
}

/// 加密：输出格式为 `nonce(12B) || ciphertext`。
pub fn encrypt(plain: &[u8]) -> Result<Vec<u8>> {
    let key = load_or_create_key()?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));

    let mut nonce = [0u8; NONCE_LEN];
    rand::thread_rng().fill_bytes(&mut nonce);

    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plain)
        .map_err(|_| anyhow!("账号库加密失败"))?;

    let mut out = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// 解密，与 `encrypt` 对应。
pub fn decrypt(blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() <= NONCE_LEN {
        return Err(anyhow!("账号库文件长度异常"));
    }
    let key = load_or_create_key()?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));

    let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow!("账号库解密失败（主密钥可能已变更）"))
}
