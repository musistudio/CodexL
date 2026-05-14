use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use ring::{aead, pbkdf2};
use serde::Deserialize;
use serde_json::json;
use std::num::NonZeroU32;

const AAD: &[u8] = b"codexl-remote-e2ee-v1";
const BINARY_MAGIC: &[u8; 4] = b"CXE1";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;
const PBKDF2_ITERATIONS: u32 = 150_000;
const SALT_PREFIX: &str = "codexl-remote-e2ee-v1";

#[derive(Clone, Debug)]
pub(crate) struct RemoteCrypto {
    key: [u8; KEY_BYTES],
}

#[derive(Deserialize)]
struct EncryptedTextEnvelope {
    #[serde(rename = "type")]
    envelope_type: String,
    version: u8,
    nonce: String,
    payload: String,
}

impl RemoteCrypto {
    pub(crate) fn from_password(
        password: Option<&str>,
        token: &str,
    ) -> Result<Option<Self>, String> {
        let Some(password) = password.filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let mut key = [0_u8; KEY_BYTES];
        let iterations = NonZeroU32::new(PBKDF2_ITERATIONS)
            .ok_or_else(|| "invalid remote crypto iteration count".to_string())?;
        let salt = format!("{}\0{}", SALT_PREFIX, token);
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            iterations,
            salt.as_bytes(),
            password.as_bytes(),
            &mut key,
        );
        Ok(Some(Self { key }))
    }

    pub(crate) fn encrypt_text(&self, plaintext: &str) -> Result<String, String> {
        let encrypted = self.encrypt_bytes(plaintext.as_bytes())?;
        let nonce = &encrypted[BINARY_MAGIC.len()..BINARY_MAGIC.len() + NONCE_BYTES];
        let payload = &encrypted[BINARY_MAGIC.len() + NONCE_BYTES..];
        Ok(json!({
            "type": "e2ee",
            "version": 1,
            "nonce": URL_SAFE_NO_PAD.encode(nonce),
            "payload": URL_SAFE_NO_PAD.encode(payload),
        })
        .to_string())
    }

    pub(crate) fn decrypt_text(&self, ciphertext: &str) -> Result<String, String> {
        let envelope = serde_json::from_str::<EncryptedTextEnvelope>(ciphertext)
            .map_err(|_| "encrypted remote payload is required".to_string())?;
        if envelope.envelope_type != "e2ee" || envelope.version != 1 {
            return Err("unsupported encrypted remote payload".to_string());
        }
        let nonce = URL_SAFE_NO_PAD
            .decode(envelope.nonce.as_bytes())
            .map_err(|_| "invalid encrypted remote nonce".to_string())?;
        if nonce.len() != NONCE_BYTES {
            return Err("invalid encrypted remote nonce length".to_string());
        }
        let payload = URL_SAFE_NO_PAD
            .decode(envelope.payload.as_bytes())
            .map_err(|_| "invalid encrypted remote payload".to_string())?;
        let mut packet = Vec::with_capacity(BINARY_MAGIC.len() + NONCE_BYTES + payload.len());
        packet.extend_from_slice(BINARY_MAGIC);
        packet.extend_from_slice(&nonce);
        packet.extend_from_slice(&payload);
        let decrypted = self.decrypt_bytes(&packet)?;
        String::from_utf8(decrypted).map_err(|_| "encrypted remote text is not UTF-8".to_string())
    }

    pub(crate) fn encrypt_bytes(&self, plaintext: &[u8]) -> Result<Vec<u8>, String> {
        let mut nonce_bytes = [0_u8; NONCE_BYTES];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
        let key = self.less_safe_key()?;
        let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
        let mut in_out = plaintext.to_vec();
        key.seal_in_place_append_tag(nonce, aead::Aad::from(AAD), &mut in_out)
            .map_err(|_| "failed to encrypt remote payload".to_string())?;

        let mut packet = Vec::with_capacity(BINARY_MAGIC.len() + NONCE_BYTES + in_out.len());
        packet.extend_from_slice(BINARY_MAGIC);
        packet.extend_from_slice(&nonce_bytes);
        packet.extend_from_slice(&in_out);
        Ok(packet)
    }

    pub(crate) fn decrypt_bytes(&self, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
        if ciphertext.len() < BINARY_MAGIC.len() + NONCE_BYTES
            || &ciphertext[..BINARY_MAGIC.len()] != BINARY_MAGIC
        {
            return Err("encrypted remote binary payload is required".to_string());
        }
        let mut nonce_bytes = [0_u8; NONCE_BYTES];
        nonce_bytes
            .copy_from_slice(&ciphertext[BINARY_MAGIC.len()..BINARY_MAGIC.len() + NONCE_BYTES]);
        let mut in_out = ciphertext[BINARY_MAGIC.len() + NONCE_BYTES..].to_vec();
        let key = self.less_safe_key()?;
        let nonce = aead::Nonce::assume_unique_for_key(nonce_bytes);
        let decrypted = key
            .open_in_place(nonce, aead::Aad::from(AAD), &mut in_out)
            .map_err(|_| "failed to decrypt remote payload".to_string())?;
        Ok(decrypted.to_vec())
    }

    fn less_safe_key(&self) -> Result<aead::LessSafeKey, String> {
        let unbound = aead::UnboundKey::new(&aead::AES_256_GCM, &self.key)
            .map_err(|_| "failed to initialize remote crypto".to_string())?;
        Ok(aead::LessSafeKey::new(unbound))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        let crypto = RemoteCrypto::from_password(Some("secret"), "token")
            .unwrap()
            .unwrap();
        let encrypted = crypto.encrypt_text(r#"{"type":"refresh"}"#).unwrap();
        assert_ne!(encrypted, r#"{"type":"refresh"}"#);
        assert_eq!(
            crypto.decrypt_text(&encrypted).unwrap(),
            r#"{"type":"refresh"}"#
        );
    }

    #[test]
    fn binary_round_trip() {
        let crypto = RemoteCrypto::from_password(Some("secret"), "token")
            .unwrap()
            .unwrap();
        let encrypted = crypto.encrypt_bytes(b"frame-bytes").unwrap();
        assert_ne!(encrypted, b"frame-bytes");
        assert_eq!(crypto.decrypt_bytes(&encrypted).unwrap(), b"frame-bytes");
    }

    #[test]
    fn password_spaces_are_significant() {
        let crypto = RemoteCrypto::from_password(Some(" secret"), "token")
            .unwrap()
            .unwrap();
        let other = RemoteCrypto::from_password(Some("secret"), "token")
            .unwrap()
            .unwrap();
        let encrypted = crypto.encrypt_text("payload").unwrap();
        assert!(other.decrypt_text(&encrypted).is_err());
    }
}
