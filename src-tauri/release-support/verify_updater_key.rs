use std::{env, fs, path::PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use minisign_verify::{PublicKey, Signature};

fn decode_public_key(public_text: &str) -> Result<PublicKey, Box<dyn std::error::Error>> {
    let public_text = public_text.trim();
    if let Ok(public_key) = PublicKey::decode(public_text) {
        return Ok(public_key);
    }
    if let Ok(public_key) = PublicKey::from_base64(public_text) {
        return Ok(public_key);
    }

    let decoded = STANDARD.decode(public_text)?;
    let decoded_text = std::str::from_utf8(&decoded)?;
    Ok(PublicKey::decode(decoded_text.trim())?)
}

fn decode_signature(signature_text: &str) -> Result<Signature, Box<dyn std::error::Error>> {
    let signature_text = signature_text.trim();
    if let Ok(signature) = Signature::decode(signature_text) {
        return Ok(signature);
    }

    let decoded = STANDARD.decode(signature_text)?;
    let decoded_text = std::str::from_utf8(&decoded)?;
    Ok(Signature::decode(decoded_text.trim())?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1).map(PathBuf::from);
    let payload_path = args.next().ok_or("payload path is required")?;
    let signature_path = args.next().ok_or("signature path is required")?;
    if args.next().is_some() {
        return Err("unexpected arguments".into());
    }
    let public_text = env::var("TAURI_UPDATER_PUBLIC_KEY")?;
    let public_key = decode_public_key(&public_text)?;
    let signature = decode_signature(&fs::read_to_string(signature_path)?)?;
    public_key.verify(&fs::read(payload_path)?, &signature, false)?;
    println!("updater signing key pair verified");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_tauri_cli_wrapped_public_key() {
        const TAURI_PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEMwRTczMzUzRDZBQUI2RDMKUldUVHRxcldVelBud1BZUWlwbHdaMnRRNFN4Uk5FY1BvTUlSeERQVDVBT0ZHWStTZlEyeDhuVEMK";

        assert!(decode_public_key(TAURI_PUBLIC_KEY).is_ok());
    }

    #[test]
    fn decodes_tauri_cli_wrapped_signature() {
        const TAURI_SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUVHRxcldVelBud0NBRmhFSnIyUHM5T1JHUy9Zd2VXTmVqamJyWnl3WFM4YTY3V3g3Z201aHpOUnR6elRnUzlINUNBNlFpTjFIYzlGZVY2TTZ4UzlvM2MzYTNKcExxbWdBPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzg4NTE1NjM4CWZpbGU6Y2FuYXJ5LnR4dApKQW9SK3JXZE1LaEJFTzIrazVZRms5WlpGS0xQa0g3VEZ5OExDaklXS0hjWDE2Mjd1SENQV0ZrT2lCT0ROVzZ6T3pSR2EvcmZEQ3BGcGZnUktNMFFCdz09Cg==";

        assert!(decode_signature(TAURI_SIGNATURE).is_ok());
    }
}
