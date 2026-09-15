use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use async_trait::async_trait;
use nexofolio_access::EmergencyPassword;
use nexofolio_contracts::{Error, Result, Secret};
use sha2::{Digest, Sha256};

pub struct SessionCrypto {
    cipher: Aes256Gcm,
}
impl SessionCrypto {
    pub fn new(key: &Secret) -> Result<Self> {
        let bytes = decode_hex(key.expose())
            .ok_or_else(|| Error::invalid("NEXOFOLIO_SESSION_KEY must be 64 hex characters"))?;
        if bytes.len() != 32 {
            return Err(Error::invalid(
                "NEXOFOLIO_SESSION_KEY must be 64 hex characters",
            ));
        }
        Ok(Self {
            cipher: Aes256Gcm::new_from_slice(&bytes).map_err(|_| Error::Unavailable {
                component: "session_crypto",
            })?,
        })
    }
    pub fn generate(&self) -> Secret {
        let bytes = Aes256Gcm::generate_key(&mut OsRng);
        Secret::new(format!(
            "nfi_{}",
            bytes.iter().map(|v| format!("{v:02x}")).collect::<String>()
        ))
    }
    pub fn hash(token: &Secret) -> Vec<u8> {
        Sha256::digest(token.expose().as_bytes()).to_vec()
    }
    pub fn encrypt(&self, token: &Secret, user: &str) -> Result<Vec<u8>> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let encrypted = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: token.expose().as_bytes(),
                    aad: user.as_bytes(),
                },
            )
            .map_err(|_| Error::Unavailable {
                component: "session_crypto",
            })?;
        Ok([&nonce[..], encrypted.as_slice()].concat())
    }
    pub fn decrypt(&self, bytes: &[u8], user: &str) -> Result<Secret> {
        if bytes.len() < 28 {
            return Err(Error::Unavailable {
                component: "session_crypto",
            });
        }
        let raw = self
            .cipher
            .decrypt(
                &Nonce::from(<[u8; 12]>::try_from(&bytes[..12]).expect("checked nonce length")),
                Payload {
                    msg: &bytes[12..],
                    aad: user.as_bytes(),
                },
            )
            .map_err(|_| Error::Unavailable {
                component: "session_crypto",
            })?;
        Ok(Secret::new(String::from_utf8(raw).map_err(|_| {
            Error::Unavailable {
                component: "session_crypto",
            }
        })?))
    }
}
fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() != 64 || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

pub struct ConfiguredEmergencyPassword {
    hash: Option<String>,
}
impl ConfiguredEmergencyPassword {
    pub fn new(hash: Option<&Secret>) -> Result<Self> {
        if let Some(hash) = hash {
            let p = PasswordHash::new(hash.expose()).map_err(|_| {
                Error::invalid("NEXOFOLIO_EMERGENCY_PASSWORD_HASH must be an Argon2id PHC hash")
            })?;
            if p.algorithm.as_str() != "argon2id" {
                return Err(Error::invalid("Emergency password hash must use Argon2id"));
            }
        }
        Ok(Self {
            hash: hash.map(|h| h.expose().to_owned()),
        })
    }
}
#[async_trait]
impl EmergencyPassword for ConfiguredEmergencyPassword {
    async fn matches(&self, password: &Secret) -> Result<bool> {
        let Some(hash) = self.hash.clone() else {
            return Ok(false);
        };
        let secret = Secret::new(password.expose());
        tokio::task::spawn_blocking(move || {
            let parsed = PasswordHash::new(&hash).map_err(|_| Error::Unavailable {
                component: "emergency_password",
            })?;
            Ok(Argon2::default()
                .verify_password(secret.expose().as_bytes(), &parsed)
                .is_ok())
        })
        .await
        .map_err(|_| Error::Unavailable {
            component: "emergency_password",
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_tokens_roundtrip_are_bound_to_user_and_reject_tampering() {
        let crypto = SessionCrypto::new(&Secret::new("ab".repeat(32))).unwrap();
        let token = crypto.generate();
        let encrypted = crypto.encrypt(&token, "user-a").unwrap();
        assert_eq!(
            crypto.decrypt(&encrypted, "user-a").unwrap().expose(),
            token.expose()
        );
        assert!(crypto.decrypt(&encrypted, "user-b").is_err());
        let mut tampered = encrypted.clone();
        tampered[15] ^= 1;
        assert!(crypto.decrypt(&tampered, "user-a").is_err());
        assert!(crypto.decrypt(&[], "user-a").is_err());
        let other = SessionCrypto::new(&Secret::new("cd".repeat(32))).unwrap();
        assert!(other.decrypt(&encrypted, "user-a").is_err());
        assert!(SessionCrypto::new(&Secret::new("not-a-key")).is_err());
    }
}
