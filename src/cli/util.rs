use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

use kzktdk::model::decrypt::decrypt_model;

/// File name of `path`, or a descriptive error.
///
/// Central replacement for `path.file_name().unwrap()`: directory/archive
/// listings always yield file names, but this error beats a panic if one
/// ever doesn't.
pub fn file_name(path: &Path) -> Result<&std::ffi::OsStr> {
    path.file_name()
        .with_context(|| format!("Path has no file name: {}", path.display()))
}

/// Load a required (latin) font: explicit file → candidate files → embedded.
///
/// Shared shape of the per-command font chains. Candidate read errors
/// propagate (`?`), matching the historical behavior.
pub fn load_font_bytes(
    query: impl AsRef<Path>,
    candidate: &str,
    embedded: &[u8],
) -> Result<Vec<u8>> {
    let query = query.as_ref();
    if query.exists() {
        return Ok(std::fs::read(query)?);
    }
    if let Some(p) = find_file_in_candidates(candidate) {
        return Ok(std::fs::read(&p)?);
    }
    Ok(embedded.to_vec())
}

/// Best-effort (CJK) variant of [`load_font_bytes`]: failures yield `None`.
pub fn load_cjk_bytes(query: impl AsRef<Path>, candidate: &str) -> Option<Vec<u8>> {
    let query = query.as_ref();
    if query.exists() {
        return std::fs::read(query).ok();
    }
    find_file_in_candidates(candidate).and_then(|p| std::fs::read(p).ok())
}

pub fn find_file_in_candidates(relative: &str) -> Option<PathBuf> {
    let direct = Path::new(relative);
    if direct.exists() {
        return Some(direct.to_path_buf());
    }

    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let candidate = parent.join(relative);
        if candidate.exists() {
            return Some(candidate);
        }
        if let Some(grandparent) = parent.parent() {
            let candidate2 = grandparent.join(relative);
            if candidate2.exists() {
                return Some(candidate2);
            }
        }
    }

    if let Ok(data_home) = std::env::var("XDG_DATA_HOME") {
        let candidate = PathBuf::from(data_home).join("kzktdk").join(relative);
        if candidate.exists() {
            return Some(candidate);
        }
    } else if let Ok(home) = std::env::var("HOME") {
        let candidate = PathBuf::from(home)
            .join(".local/share/kzktdk")
            .join(relative);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
        let candidate = PathBuf::from(local_appdata).join("kzktdk").join(relative);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    if let Ok(appdata) = std::env::var("APPDATA") {
        let candidate = PathBuf::from(appdata).join("kzktdk").join(relative);
        if candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

pub fn ensure_model(model_path: &Path) -> Result<PathBuf> {
    if model_path.exists() {
        return Ok(model_path.to_path_buf());
    }

    if let Some(onnx) = find_file_in_candidates(kzktdk::config::DEFAULT_MODEL_PATH) {
        return Ok(onnx);
    }

    if let Some(dat) = find_file_in_candidates(kzktdk::config::DEFAULT_MODEL_DAT_PATH) {
        println!(
            "[Info] ONNX model not found at {:?}. Auto-decrypting from {:?}...",
            model_path, dat
        );
        let dest = dat.with_extension("onnx");
        return decrypt_model(&dat, &dest);
    }

    bail!(
        "Model file not found at {:?}. Please run `kzktdk decrypt-model` or place kzkt.dat in models/",
        model_path
    )
}

pub fn parse_jobs(jobs_str: &str) -> usize {
    if jobs_str.eq_ignore_ascii_case("auto") {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(kzktdk::config::DEFAULT_JOBS_FALLBACK)
    } else if let Ok(n) = jobs_str.parse::<usize>() {
        n.max(1)
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(kzktdk::config::DEFAULT_JOBS_FALLBACK)
    }
}

pub fn print_developer_help() {
    println!(
        r#"================================================================================
                    KZKT-DK — DEVELOPER REFERENCE MANUAL
================================================================================

1. INTERNAL & DEVELOPER SUBCOMMANDS (Hidden from public --help):

   • kzktdk decrypt-model [--source <PATH>] [--dest <PATH>]
     Decrypts the bundled obfuscated model file (models/kzkt.dat -> models/kzkt.onnx).
     Algorithm: Streaming byte-wise XOR 0x5A (90), verified via 0x08 Protobuf header.

   • kzktdk detect <INPUT> [-o <OUTPUT>] [-m <MODEL>]
     Runs the YOLOv8 3-stage confidence cascade on an image and draws annotated bounding boxes.
     Logs detected bubble coordinates: [x1, y1, x2, y2] with confidence scores.

   • kzktdk inpaint <INPUT> [-o <OUTPUT>] [-m <MODEL>]
     Isolates bubble interior, applies sub-pixel anti-aliasing edge dilation,
     and executes Telea Fast Marching inpainting without translation (parallel rayon).

   • kzktdk translate <INPUT> [OPTIONS]
     Full 4-stage pipeline: Detection -> Inpainting -> Mosaic LLM Query -> Typesetting.
     Options:
       --prompt <STRING>      Append custom system prompt instructions or glossary rules
       --batch-size <N>       Max dialogue bubbles grouped per LLM request (default: 15)
       -t, --target-lang      Target language (default: English)
       -p, --provider         LLM provider (gemini, openai, ollama, claude)
       --fallback-provider    Comma-separated fallback providers (e.g. openai,claude)
       --rate-limit <N>       RPS limit with exponential backoff (default: 3)
       --jobs <auto|N>        Parallel jobs for batch (default: auto = num_cpus)
       --no-cache             Disable translation cache
       --clear-cache          Clear cache and exit

2. PIPELINE INTERNALS & THRESHOLDS:

   • YOLOv8 Detection:
     - Input size: 640x640 letterbox normalized NCHW
     - Stage 1: conf >= 0.28, iou = 0.45
     - Stage 2: conf >= 0.18, iou = 0.55
     - Stage 3: conf >= 0.10, iou = 0.65

   • Cache: blake3 hash of crop bytes + target_lang + provider + model + prompt_sig (rusqlite)

   • Inpainting: rayon parallel per bubble

   • Batch: tokio JoinSet + Semaphore(jobs) parallel pages

   • EPUB: treated as ZIP, META-INF skipped

================================================================================
"#
    );
}
