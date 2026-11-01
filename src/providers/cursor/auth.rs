//! Cursor's desktop token codec. Decrypt in memory; never change its stores.
use anyhow::{Result, anyhow, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::path::Path;

const PREFIX: &str = "cursor-safe-storage:v1:";

pub(super) fn decode(value: &str, home: &Path) -> Result<String> {
    ensure!(
        value.len() <= crate::input::AUTH_BYTES,
        "Cursor login is too large"
    );
    let value = serde_json::from_str::<String>(value).unwrap_or_else(|_| value.to_owned());
    let value = value.trim();
    let Some(encoded) = value.strip_prefix(PREFIX) else {
        if value.starts_with("cursor-safe-storage:") {
            bail!("Cursor uses an unsupported login encryption version");
        }
        return Ok(value.to_owned());
    };
    let encrypted = STANDARD
        .decode(encoded)
        .map_err(|_| anyhow!("Cursor's encrypted login is unreadable"))?;
    decrypt(&encrypted, home).map(|value| value.trim().to_owned())
}

#[cfg(windows)]
fn decrypt(encrypted: &[u8], home: &Path) -> Result<String> {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};

    // Electron safeStorage on Windows: v10 || nonce (12) || ciphertext || tag (16).
    ensure!(
        encrypted.starts_with(b"v10") && encrypted.len() >= 31,
        "Cursor uses an unsupported Windows login cipher"
    );
    let state = super::super::http::read(&home.join("Local State"))
        .ok_or_else(|| anyhow!("Cursor's login encryption settings could not be read"))?;
    let encoded_key = state
        .pointer("/os_crypt/encrypted_key")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow!("Cursor's login encryption key is unavailable"))?;
    let wrapped = STANDARD
        .decode(encoded_key)
        .map_err(|_| anyhow!("Cursor's login encryption key is unreadable"))?;
    let wrapped = wrapped
        .strip_prefix(b"DPAPI")
        .ok_or_else(|| anyhow!("Cursor uses an unsupported Windows key format"))?;
    let mut key = unprotect(wrapped)?;
    let unbound = UnboundKey::new(&AES_256_GCM, &key);
    key.fill(0);
    let key =
        LessSafeKey::new(unbound.map_err(|_| anyhow!("Cursor's login encryption key is invalid"))?);
    let nonce: [u8; 12] = encrypted[3..15].try_into().unwrap();
    let mut ciphertext = encrypted[15..].to_vec();
    let plaintext = key
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::empty(),
            &mut ciphertext,
        )
        .map_err(|_| anyhow!("Cursor's login could not be decrypted with this Windows account"))?;
    let access = std::str::from_utf8(plaintext)
        .map(str::to_owned)
        .map_err(|_| anyhow!("Cursor's decrypted login is unreadable"));
    ciphertext.fill(0);
    access
}

#[cfg(windows)]
fn unprotect(encrypted: &[u8]) -> Result<Vec<u8>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptUnprotectData,
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(encrypted.len())?,
        pbData: encrypted.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // DPAPI allocates output for the current Windows user. Copy it into owned
    // memory, clear the allocation, then release it using LocalFree.
    unsafe {
        ensure!(
            CryptUnprotectData(
                &input,
                null_mut(),
                null(),
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output
            ) != 0,
            "Windows could not unlock Cursor's login encryption key"
        );
        if output.pbData.is_null() {
            bail!("Windows returned an empty Cursor login encryption key");
        }
        let data = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let owned = data.to_vec();
        data.fill(0);
        LocalFree(output.pbData.cast());
        Ok(owned)
    }
}

#[cfg(not(windows))]
fn decrypt(_encrypted: &[u8], _home: &Path) -> Result<String> {
    bail!(
        "Cursor's encrypted desktop login is not supported on this platform. Sign in to Cursor CLI or set CURSOR_AUTH_TOKEN."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plaintext_and_quoted_logins_remain_supported() {
        assert_eq!(decode(" token ", Path::new(".")).unwrap(), "token");
        assert_eq!(decode(r#""token""#, Path::new(".")).unwrap(), "token");
        assert_eq!(decode("", Path::new(".")).unwrap(), "");
    }

    #[test]
    fn invalid_wrappers_never_become_bearer_tokens_or_appear_in_errors() {
        for value in [
            "cursor-safe-storage:v1:invalid-secret!",
            "cursor-safe-storage:v2:secret",
        ] {
            let error = decode(value, Path::new(".")).unwrap_err().to_string();
            assert!(!error.contains("secret"));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_safe_storage_round_trip_and_tampering() {
        use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
        use std::ptr::null;
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::Cryptography::{
                CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
            },
        };

        let root = tempfile::tempdir().unwrap();
        let key = [0x42; 32];
        let input = CRYPT_INTEGER_BLOB {
            cbData: key.len() as u32,
            pbData: key.as_ptr().cast_mut(),
        };
        let mut output = CRYPT_INTEGER_BLOB::default();
        let mut wrapped = b"DPAPI".to_vec();
        unsafe {
            assert_ne!(
                CryptProtectData(
                    &input,
                    null(),
                    null(),
                    null(),
                    null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output
                ),
                0
            );
            assert!(!output.pbData.is_null());
            wrapped.extend_from_slice(std::slice::from_raw_parts(
                output.pbData,
                output.cbData as usize,
            ));
            LocalFree(output.pbData.cast());
        }
        std::fs::write(
            root.path().join("Local State"),
            serde_json::to_vec(&serde_json::json!({
                "os_crypt": {"encrypted_key": STANDARD.encode(&wrapped)}
            }))
            .unwrap(),
        )
        .unwrap();
        let cipher = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &key).unwrap());
        let nonce = [0x24; 12];
        let mut ciphertext = b"synthetic-test-token".to_vec();
        cipher
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut ciphertext,
            )
            .unwrap();
        let mut payload = b"v10".to_vec();
        payload.extend_from_slice(&nonce);
        payload.extend_from_slice(&ciphertext);
        let encoded = format!("{PREFIX}{}", STANDARD.encode(&payload));
        assert_eq!(
            decode(&encoded, root.path()).unwrap(),
            "synthetic-test-token"
        );
        assert_eq!(
            decode(&serde_json::to_string(&encoded).unwrap(), root.path()).unwrap(),
            "synthetic-test-token"
        );
        *payload.last_mut().unwrap() ^= 1;
        let tampered = format!("{PREFIX}{}", STANDARD.encode(&payload));
        assert!(decode(&tampered, root.path()).is_err());
        let unsupported = format!("{PREFIX}{}", STANDARD.encode(b"v20unsupported"));
        assert!(decode(&unsupported, root.path()).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn encrypted_logins_offer_a_supported_alternative() {
        let encoded = format!("{PREFIX}{}", STANDARD.encode(b"encrypted"));
        let error = decode(&encoded, Path::new(".")).unwrap_err().to_string();
        assert!(error.contains("Cursor CLI"));
        assert!(error.contains("CURSOR_AUTH_TOKEN"));
    }
}
