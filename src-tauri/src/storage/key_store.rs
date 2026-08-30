use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand::{rngs::OsRng, RngCore};
use std::sync::Mutex;
use zeroize::{Zeroize, Zeroizing};

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
            Ok(mut encoded) => {
                let decoded = STANDARD
                    .decode(encoded.as_bytes())
                    .map_err(|_| KeyStoreError::InvalidKey)?;
                encoded.zeroize();
                let key: [u8; 32] = decoded.try_into().map_err(|_| KeyStoreError::InvalidKey)?;
                Ok(Zeroizing::new(key))
            }
            Err(keyring::Error::NoEntry) => {
                let mut key = [0_u8; 32];
                OsRng.fill_bytes(&mut key);
                let mut encoded = STANDARD.encode(key);
                if entry.set_password(&encoded).is_err() {
                    key.zeroize();
                    encoded.zeroize();
                    return Err(KeyStoreError::SecureStorageUnavailable);
                }
                encoded.zeroize();
                Ok(Zeroizing::new(key))
            }
            Err(_) => Err(KeyStoreError::SecureStorageUnavailable),
        }
    }
}

pub struct MemoryKeyStore(Mutex<Zeroizing<[u8; 32]>>);

impl MemoryKeyStore {
    pub fn new(key: [u8; 32]) -> Self {
        Self(Mutex::new(Zeroizing::new(key)))
    }
}

impl KeyStore for MemoryKeyStore {
    fn load_or_create(&self) -> Result<Zeroizing<[u8; 32]>, KeyStoreError> {
        let key = self.0.lock().unwrap_or_else(|p| p.into_inner());
        Ok(Zeroizing::new(**key))
    }
}
