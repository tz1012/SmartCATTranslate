use aes_gcm::{
    aead::{Aead, Payload},
    Aes256Gcm, KeyInit, Nonce,
};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

const ENVELOPE_VERSION: u8 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EncryptedEnvelope {
    pub version: u8,
    pub nonce: [u8; 12],
    pub ciphertext: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    #[error("unsupported encrypted envelope version")]
    UnsupportedVersion,
    #[error("authenticated encryption failed")]
    Authentication,
    #[error("encrypted envelope encoding failed")]
    Encoding,
}

pub struct CryptoBox {
    key: Zeroizing<[u8; 32]>,
}

impl CryptoBox {
    pub fn from_key(key: [u8; 32]) -> Self {
        Self {
            key: Zeroizing::new(key),
        }
    }

    pub fn seal(&self, plaintext: &[u8], aad: &[u8]) -> Result<EncryptedEnvelope, CryptoError> {
        let mut nonce = [0_u8; 12];
        OsRng.fill_bytes(&mut nonce);
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref())
            .map_err(|_| CryptoError::Authentication)?;
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| CryptoError::Authentication)?;
        Ok(EncryptedEnvelope {
            version: ENVELOPE_VERSION,
            nonce,
            ciphertext,
        })
    }

    pub fn open(&self, envelope: &EncryptedEnvelope, aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if envelope.version != ENVELOPE_VERSION {
            return Err(CryptoError::UnsupportedVersion);
        }
        let cipher = Aes256Gcm::new_from_slice(self.key.as_ref())
            .map_err(|_| CryptoError::Authentication)?;
        cipher
            .decrypt(
                Nonce::from_slice(&envelope.nonce),
                Payload {
                    msg: &envelope.ciphertext,
                    aad,
                },
            )
            .map_err(|_| CryptoError::Authentication)
    }

    pub fn seal_json<T: Serialize>(&self, value: &T, aad: &[u8]) -> Result<Vec<u8>, CryptoError> {
        let mut plaintext = serde_json::to_vec(value).map_err(|_| CryptoError::Encoding)?;
        let result = self
            .seal(&plaintext, aad)
            .and_then(|value| serde_json::to_vec(&value).map_err(|_| CryptoError::Encoding));
        plaintext.zeroize();
        result
    }

    pub fn open_json<T: for<'de> Deserialize<'de>>(
        &self,
        encoded: &[u8],
        aad: &[u8],
    ) -> Result<T, CryptoError> {
        let envelope: EncryptedEnvelope =
            serde_json::from_slice(encoded).map_err(|_| CryptoError::Encoding)?;
        let mut plaintext = self.open(&envelope, aad)?;
        let result = serde_json::from_slice(&plaintext).map_err(|_| CryptoError::Encoding);
        plaintext.zeroize();
        result
    }
}
