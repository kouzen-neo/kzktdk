//! Tauri-ready GUI bindings (feature `tauri`).
//!
//! These are plain sync functions returning `Result<T, String>` so a Tauri
//! frontend can attach `#[tauri::command]` to them directly:
//!
//! ```ignore
//! #[tauri::command]
//! fn editor_load_page(path: String) -> Result<PageEditData, String> {
//!     kzktdk::tauri::editor_load_page(&path)
//! }
//! ```
//!
//! Long-lived sessions (fonts + YOLO loaded once) should be kept in Tauri
//! managed state instead, e.g. `Mutex<EditorSession>`:
//!
//! ```ignore
//! struct GuiState(std::sync::Mutex<kzktdk::editor::EditorSession>);
//! ```
//!
//! Rules: no `println!`, no `process::exit`, owned data or base64 PNG only.

use std::path::{Path, PathBuf};

use crate::editor::{EditPatch, EditorSession, apply_patch, png_base64, thumbnail};
use crate::metadata::{PageEditData, save_page_metadata};

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn session_for(font: Option<&str>, cjk_font: Option<&str>) -> Result<EditorSession, String> {
    let font_bytes = match font {
        Some(p) if Path::new(p).is_file() => std::fs::read(p).map_err(err)?,
        _ => include_bytes!("../fonts/Komika Axis.ttf").to_vec(),
    };
    let cjk_bytes = match cjk_font {
        Some(p) if Path::new(p).is_file() => std::fs::read(p).ok(),
        _ => std::fs::read("fonts/KosugiMaru.ttf").ok(),
    };
    EditorSession::new(font_bytes, cjk_bytes).map_err(err)
}

/// Load a `.kedit.json` page for the editor.
pub fn editor_load_page(path: &str) -> Result<PageEditData, String> {
    EditorSession::new(include_bytes!("../fonts/Komika Axis.ttf").to_vec(), None)
        .map_err(err)?
        .load_page(Path::new(path))
        .map_err(err)
}

/// Apply a transactional patch (same semantics as `--stdin-patch`).
/// Returns human-readable log lines; any error aborts without writing.
pub fn editor_apply_patch(path: &str, patch: EditPatch) -> Result<Vec<String>, String> {
    let p = Path::new(path);
    let mut trial = EditorSession::new(include_bytes!("../fonts/Komika Axis.ttf").to_vec(), None)
        .map_err(err)?
        .load_page(p)
        .map_err(err)?;
    let (logs, errors) = apply_patch(&mut trial, &patch);
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    save_page_metadata(p, &trial).map_err(err)?;
    Ok(logs)
}

/// Render a full page to base64 PNG (optionally thumbnailed).
pub fn editor_render_page(
    image: &str,
    metadata: &str,
    font: Option<&str>,
    cjk_font: Option<&str>,
    thumb: u32,
) -> Result<String, String> {
    let session = session_for(font, cjk_font)?;
    let data = session.load_page(Path::new(metadata)).map_err(err)?;
    let img = image::open(image).map_err(err)?.to_rgb8();
    let mut rgb = session.render_page(&img, &data).map_err(err)?;
    if thumb > 0 {
        rgb = thumbnail(&rgb, thumb);
    }
    png_base64(&rgb).map_err(err)
}

/// Preview a single bubble to base64 PNG (optionally thumbnailed).
pub fn editor_preview_bubble(
    image: &str,
    metadata: &str,
    id: &str,
    text: Option<&str>,
    font: Option<&str>,
    cjk_font: Option<&str>,
    thumb: u32,
) -> Result<String, String> {
    let session = session_for(font, cjk_font)?;
    let data = session.load_page(Path::new(metadata)).map_err(err)?;
    let img = image::open(image).map_err(err)?.to_rgb8();
    let mut rgb = session
        .preview_bubble(&img, &data, id, text, None)
        .map_err(err)?;
    if thumb > 0 {
        rgb = thumbnail(&rgb, thumb);
    }
    png_base64(&rgb).map_err(err)
}

/// Run detection on an image file and return an empty `PageEditData`.
/// Requires the ONNX model path (loaded fresh per call; prefer a shared
/// `EditorSession::open_model` for repeated calls).
pub fn editor_detect(image: &str, model: &str) -> Result<PageEditData, String> {
    let session = EditorSession::new(include_bytes!("../fonts/Komika Axis.ttf").to_vec(), None)
        .map_err(err)?;
    let _ = session; // fonts validated; detection needs a model session below
    let mut det_session = EditorSession::open_model(
        Path::new(model),
        include_bytes!("../fonts/Komika Axis.ttf").to_vec(),
        None,
    )
    .map_err(err)?;
    det_session.export_page(Path::new(image)).map_err(err)
}

/// Pack rendered images into a CBZ archive. Returns the output path.
pub fn editor_pack_chapter(images: Vec<String>, output: &str) -> Result<String, String> {
    let paths: Vec<PathBuf> = images.iter().map(PathBuf::from).collect();
    crate::archive::create_cbz(&paths, Path::new(output)).map_err(err)?;
    Ok(output.to_string())
}
