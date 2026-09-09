use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

const MODEL_DECRYPT_KEY_STR: &str = "indravoyager";

/// Computes the XOR key: "indravoyager".len() * 7 + 6 = 90 (0x5A)
pub fn get_decrypt_key() -> u8 {
    (MODEL_DECRYPT_KEY_STR.len() * 7 + 6) as u8
}

/// Checks if the file header starts with byte 0x08 (ONNX ModelProto field 1: ir_version).
pub fn looks_like_onnx(path: &Path) -> bool {
    if let Ok(mut file) = File::open(path) {
        let mut header = [0u8; 1];
        if file.read_exact(&mut header).is_ok() {
            return header[0] == 0x08;
        }
    }
    false
}

/// Decrypts a `.dat` model using XOR key into an `.onnx` model file.
/// If `dest` already exists and is a valid ONNX file, decryption is skipped.
pub fn decrypt_model(source: &Path, dest: &Path) -> Result<PathBuf> {
    if dest.exists() && looks_like_onnx(dest) {
        return Ok(dest.to_path_buf());
    }

    if !source.exists() {
        bail!("Model source file not found: {:?}", source);
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {:?}", parent))?;
    }

    let key = get_decrypt_key();
    let src_file =
        File::open(source).with_context(|| format!("Failed to open model source {:?}", source))?;
    let mut reader = BufReader::with_capacity(64 * 1024, src_file);

    let temp_dest = dest.with_extension(format!("tmp_{}", std::process::id()));
    let dst_file = File::create(&temp_dest)
        .with_context(|| format!("Failed to create temp destination {:?}", temp_dest))?;
    let mut writer = BufWriter::with_capacity(64 * 1024, dst_file);

    let mut buffer = [0u8; 64 * 1024];
    let mut total_bytes = 0usize;

    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        for byte in &mut buffer[..read] {
            *byte ^= key;
        }
        writer.write_all(&buffer[..read])?;
        total_bytes += read;
    }

    writer.flush()?;
    drop(writer);

    if !looks_like_onnx(&temp_dest) {
        let _ = std::fs::remove_file(&temp_dest);
        bail!(
            "Decrypted model at {:?} does not start with ONNX protobuf marker (0x08)",
            temp_dest
        );
    }

    std::fs::rename(&temp_dest, dest)
        .with_context(|| format!("Failed to rename {:?} to {:?}", temp_dest, dest))?;

    println!("Decrypted {} MB -> {:?}", total_bytes / (1024 * 1024), dest);
    Ok(dest.to_path_buf())
}
