use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use std::sync::Mutex;
use zeroize::Zeroizing;

const SERVICE: &str = "com.smartcat.translate";
const ACCOUNT: &str = "local-data-key";

#[derive(Debug, thiserror::Error)]
pub enum KeyStoreError {
    #[error("secure OS credential storage is unavailable; enable Windows Credential Manager or unlock macOS Keychain")]
    SecureStorageUnavailable,
    #[error("the stored local data key is invalid")]
    InvalidKey,
}

pub trait KeyStore: Send + Sync {
    fn load_or_create(&self) -> Result<Zeroizing<[u8; 32]>, KeyStoreError>;
}

#[derive(Default)]
pub struct OsKeyStore;

impl KeyStore for OsKeyStore {
    fn load_or_create(&self) -> Result<Zeroizing<[u8; 32]>, KeyStoreError> {
        let entry = keyring::Entry::new(SERVICE, ACCOUNT)
            .map_err(|_| KeyStoreError::SecureStorageUnavailable)?;
        match entry.get_password() {
            Ok(encoded) => {
                let encoded = Zeroizing::new(encoded);
                let decoded = Zeroizing::new(
                    STANDARD
                        .decode(encoded.as_bytes())
                        .map_err(|_| KeyStoreError::InvalidKey)?,
                );
                if decoded.len() != 32 {
                    return Err(KeyStoreError::InvalidKey);
                }
                let mut key = Zeroizing::new([0_u8; 32]);
                key.copy_from_slice(decoded.as_slice());
                Ok(key)
            }
            Err(keyring::Error::NoEntry) => {
                let mut key = Zeroizing::new([0_u8; 32]);
                OsRng.fill_bytes(&mut key);
                let encoded = Zeroizing::new(STANDARD.encode(key.as_ref()));
                if entry.set_password(&encoded).is_err() {
                    return Err(KeyStoreError::SecureStorageUnavailable);
                }
                Ok(key)
            }
            Err(_) => Err(KeyStoreError::SecureStorageUnavailable),
        }
    }
}

pub struct MemoryKeyStore(Mutex<Zeroizing<[u8; 32]>>);

impl MemoryKeyStore {
    pub fn new(key: Zeroizing<[u8; 32]>) -> Self {
        Self(Mutex::new(key))
    }
}

impl KeyStore for MemoryKeyStore {
    fn load_or_create(&self) -> Result<Zeroizing<[u8; 32]>, KeyStoreError> {
        let key = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let mut owned = Zeroizing::new([0_u8; 32]);
        owned.copy_from_slice(key.as_ref());
        Ok(owned)
    }
}
