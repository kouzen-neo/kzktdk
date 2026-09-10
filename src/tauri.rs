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
//!
//! Batch translation (`editor_translate_batch`) reuses the CLI pipeline
//! sequentially with one shared YOLO model and reports progress through a
//! caller callback (`ProgressEvent` = `pipeline::PageEvent`).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::cache::{TranslationCache, prompt_signature};
use crate::editor::{EditPatch, EditorSession, apply_patch, png_base64, thumbnail};
use crate::metadata::{PageEditData, save_page_metadata};
use crate::model::yolo::YoloModel;
use crate::pipeline::{PageEvent, TranslationContext, build_provider, translate_page};
use crate::translation::{
    ProviderChain, RateLimiter, build_translation_prompt_with_glossary, load_glossary,
};

/// Progress event for GUI batch translation. Re-exports the same event type
/// the CLI `--progress jsonl` stream is built from (`PageEvent`).
///
/// `phase` vocabulary: `detect_end`, `translate_end`, `render_end` per page,
/// then terminal `done` (success) or `failed` (`error` set).
pub type ProgressEvent = PageEvent;

/// Per-page outcome of [`editor_translate_batch`]. Setup failures (bad config,
/// provider, glossary, model, fonts) return `Err` before any page runs;
/// per-page LLM/render failures are recorded here with the original copied.
#[derive(Debug, Clone, Serialize)]
pub struct PageResult {
    pub input: String,
    pub output: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// Config for [`editor_translate_batch`]. Mirrors the essential `translate`
/// flags; use `..Default::default()` and override what you need.
#[derive(Debug, Clone)]
pub struct TranslateConfig {
    pub target_lang: String,
    pub provider: String,
    pub gemini_key: Option<String>,
    pub gemini_model: String,
    pub openai_key: Option<String>,
    pub openai_base_url: String,
    pub openai_model: String,
    pub claude_key: Option<String>,
    pub claude_model: String,
    pub fallback_provider: Option<String>,
    pub rate_limit: u32,
    pub batch_size: usize,
    pub model: String,
    pub font: Option<String>,
    pub cjk_font: Option<String>,
    pub glossary: Option<String>,
    pub custom_prompt: Option<String>,
    pub use_cache: bool,
    pub save_metadata: bool,
    pub translate_free_text: bool,
    pub ocr: String,
    pub ocr_script: String,
    pub mode: String,
}

impl Default for TranslateConfig {
    fn default() -> Self {
        Self {
            target_lang: "English".to_string(),
            provider: "gemini".to_string(),
            gemini_key: None,
            gemini_model: "gemini-2.5-flash".to_string(),
            openai_key: None,
            openai_base_url: crate::config::OPENAI_DEFAULT_BASE_URL.to_string(),
            openai_model: "gpt-4o-mini".to_string(),
            claude_key: None,
            claude_model: "claude-sonnet-4-5".to_string(),
            fallback_provider: None,
            rate_limit: 3,
            batch_size: 4,
            model: "models/kzkt.onnx".to_string(),
            font: None,
            cjk_font: None,
            glossary: None,
            custom_prompt: None,
            use_cache: true,
            save_metadata: true,
            translate_free_text: false,
            ocr: "none".to_string(),
            ocr_script: "ja".to_string(),
            mode: "vision".to_string(),
        }
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn load_font_bytes(
    font: Option<&str>,
    cjk_font: Option<&str>,
) -> Result<(Vec<u8>, Option<Vec<u8>>), String> {
    let font_bytes = match font {
        Some(p) if Path::new(p).is_file() => std::fs::read(p).map_err(err)?,
        _ => include_bytes!("../fonts/Komika Axis.ttf").to_vec(),
    };
    let cjk_bytes = match cjk_font {
        Some(p) if Path::new(p).is_file() => std::fs::read(p).ok(),
        _ => std::fs::read("fonts/KosugiMaru.ttf").ok(),
    };
    Ok((font_bytes, cjk_bytes))
}

fn session_for(font: Option<&str>, cjk_font: Option<&str>) -> Result<EditorSession, String> {
    let (font_bytes, cjk_bytes) = load_font_bytes(font, cjk_font)?;
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

/// Translate a batch of pages sequentially with one shared YOLO model,
/// reusing the exact CLI pipeline (`translate_page` + `TranslationContext`).
/// Progress (per-phase + terminal `done`/`failed`) flows through `on_event`;
/// nothing is printed and the process never exits.
///
/// `on_event` requires `Send + Sync` (not just `Send`) because it is shared
/// through the pipeline's `TranslationContext.events`, which must stay
/// `Send`-crossable for the CLI parallel path. Sequential execution here
/// satisfies the bound trivially; typical GUI sinks (channel senders,
/// `Mutex<Vec<_>>`) already are `Send + Sync`.
///
/// Setup failures (missing image, provider keys, glossary, model, fonts)
/// return `Err` before any page runs. Per-page failures copy the original
/// (same as CLI) and are recorded in the returned [`PageResult`] list.
pub fn editor_translate_batch(
    image_paths: Vec<String>,
    out_dir: &str,
    config: TranslateConfig,
    on_event: impl Fn(ProgressEvent) + Send + Sync,
) -> Result<Vec<PageResult>, String> {
    if image_paths.is_empty() {
        return Err("no input images".to_string());
    }
    let out_path = Path::new(out_dir);
    std::fs::create_dir_all(out_path).map_err(err)?;
    let mut inputs = Vec::with_capacity(image_paths.len());
    for p in &image_paths {
        let ip = PathBuf::from(p);
        if !ip.is_file() {
            return Err(format!("input image not found: {}", p));
        }
        inputs.push(ip);
    }

    let primary = build_provider(
        &config.provider,
        &config.gemini_key,
        &config.gemini_model,
        &config.openai_key,
        &config.openai_base_url,
        &config.openai_model,
        &config.claude_key,
        &config.claude_model,
    )
    .map_err(err)?;
    let mut fallbacks = Vec::new();
    if let Some(fb) = config.fallback_provider.clone() {
        for name in fb.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
            if name.eq_ignore_ascii_case(&config.provider) {
                continue;
            }
            if let Ok(p) = build_provider(
                name,
                &config.gemini_key,
                &config.gemini_model,
                &config.openai_key,
                &config.openai_base_url,
                &config.openai_model,
                &config.claude_key,
                &config.claude_model,
            ) {
                fallbacks.push(p);
            }
        }
    }
    let chain = ProviderChain { primary, fallbacks };
    let limiter = RateLimiter::new(config.rate_limit);

    let glossary_map = match config.glossary.as_deref() {
        Some(g) => Some(load_glossary(Path::new(g)).map_err(err)?),
        None => None,
    };
    let (font_bytes, cjk_bytes) =
        load_font_bytes(config.font.as_deref(), config.cjk_font.as_deref())?;
    let mut yolo = YoloModel::new(&config.model).map_err(err)?;

    let mut final_prompt =
        build_translation_prompt_with_glossary(&config.target_lang, glossary_map.as_ref());
    if let Some(custom) = config.custom_prompt.as_deref() {
        final_prompt.push_str("\n\nADDITIONAL TRANSLATION RULES:\n");
        final_prompt.push_str(custom);
    }
    let prompt_sig = prompt_signature(config.custom_prompt.as_deref());
    let cache = if config.use_cache {
        TranslationCache::open().ok()
    } else {
        None
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(err)?;
    let total = inputs.len();
    let mut results = Vec::with_capacity(total);
    for (idx, input) in inputs.iter().enumerate() {
        let file_name = input
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let target = out_path.join(&file_name);
        let sink = |ev: PageEvent| on_event(ev);
        let ctx = TranslationContext {
            events: Some(&sink),
            chain: &chain,
            rate_limiter: &limiter,
            prompt: &final_prompt,
            prompt_sig: &prompt_sig,
            target_lang: &config.target_lang,
            font_bytes: &font_bytes,
            cjk_font_bytes: cjk_bytes.as_deref(),
            batch_size: config.batch_size,
            cache: cache.as_ref(),
            save_metadata: config.save_metadata,
            metadata_dir: Some(out_path.to_path_buf()),
            translate_free_text: config.translate_free_text,
            ocr: config.ocr.clone(),
            ocr_script: config.ocr_script.clone(),
            mode: config.mode.clone(),
            ocr_model: None,
            glossary: glossary_map.clone(),
            gloss_hits: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            gloss_misses: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            progress: String::new(),
            quiet: true,
            page_idx: idx + 1,
            page_total: total,
        };
        let outcome = rt.block_on(translate_page(input, &target, &mut yolo, &ctx));
        match outcome {
            Ok(()) => {
                on_event(PageEvent {
                    idx: idx + 1,
                    total,
                    page: file_name.clone(),
                    phase: "done",
                    error: None,
                });
                results.push(PageResult {
                    input: input.to_string_lossy().to_string(),
                    output: target.to_string_lossy().to_string(),
                    ok: true,
                    error: None,
                });
            }
            Err(e) => {
                let msg = e.to_string();
                let _ = std::fs::copy(input, &target);
                on_event(PageEvent {
                    idx: idx + 1,
                    total,
                    page: file_name.clone(),
                    phase: "failed",
                    error: Some(msg.clone()),
                });
                results.push(PageResult {
                    input: input.to_string_lossy().to_string(),
                    output: target.to_string_lossy().to_string(),
                    ok: false,
                    error: Some(msg),
                });
            }
        }
    }
    Ok(results)
}
