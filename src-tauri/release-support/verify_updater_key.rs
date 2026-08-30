use std::{env, fs, path::PathBuf};

use minisign_verify::{PublicKey, Signature};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args_os().skip(1).map(PathBuf::from);
    let payload_path = args.next().ok_or("payload path is required")?;
    let signature_path = args.next().ok_or("signature path is required")?;
    if args.next().is_some() {
        return Err("unexpected arguments".into());
    }
    let public_text = env::var("TAURI_UPDATER_PUBLIC_KEY")?;
    let public_key = PublicKey::decode(public_text.trim())
        .or_else(|_| PublicKey::from_base64(public_text.trim()))?;
    let signature = Signature::decode(&fs::read_to_string(signature_path)?)?;
    public_key.verify(&fs::read(payload_path)?, &signature, false)?;
    println!("updater signing key pair verified");
    Ok(())
}
