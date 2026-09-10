//! Shared translate-page pipeline used by the CLI and GUI bindings.
//!
//! `translate_page` + `TranslationContext` used to live in the CLI binary;
//! they moved here so Tauri/GUI code (`crate::tauri`) can reuse the exact
//! same detection -> OCR -> translate -> inpaint -> typeset flow.
//!
//! Rules for this module: no `process::exit`, no unwraps on user input.
//! Terminal output happens only when `TranslationContext.events` is `None`
//! (CLI mode); GUI mode sets `events` and gets silent callbacks instead.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context, Result, bail};
use image::{GenericImageView, RgbImage};

use crate::cache::TranslationCache;
use crate::inpaint::inpaint_image;
use crate::metadata;
use crate::model::yolo::YoloModel;
use crate::translation::{
    CropItem, MosaicBuilder, Provider, ProviderChain, RateLimiter, translate_with_chain,
};
use crate::typesetting::Typesetter;

/// Progress event emitted per page phase.
/// Phases mirror the CLI `--progress jsonl` values (`detect_end`,
/// `translate_end`, `render_end`) plus terminal `done` / `failed`
/// (emitted by the batch driver, with `error` set on failure).
#[derive(Debug, Clone)]
pub struct PageEvent {
    pub idx: usize,
    pub total: usize,
    pub page: String,
    pub phase: &'static str,
    pub error: Option<String>,
}

pub struct TranslationContext<'a> {
    pub chain: &'a ProviderChain,
    pub rate_limiter: &'a RateLimiter,
    pub prompt: &'a str,
    pub prompt_sig: &'a str,
    pub target_lang: &'a str,
    pub font_bytes: &'a [u8],
    pub cjk_font_bytes: Option<&'a [u8]>,
    pub batch_size: usize,
    pub cache: Option<&'a TranslationCache>,
    pub save_metadata: bool,
    pub metadata_dir: Option<PathBuf>,
    pub translate_free_text: bool,
    pub ocr: String,
    pub ocr_script: String,
    pub mode: String,
    pub ocr_model: Option<PathBuf>,
    pub glossary: Option<BTreeMap<String, String>>,
    pub gloss_hits: Arc<AtomicUsize>,
    pub gloss_misses: Arc<AtomicUsize>,
    pub progress: String,
    pub quiet: bool,
    pub page_idx: usize,
    pub page_total: usize,
    /// Optional progress sink. When set, all terminal output
    /// (println!/eprintln!) inside `translate_page` is suppressed and
    /// every phase is delivered through this callback instead.
    /// CLI passes `None`; GUI/Tauri passes `Some`.
    pub events: Option<&'a (dyn Fn(PageEvent) + Send + Sync)>,
}

pub fn build_provider(
    name: &str,
    gemini_key: &Option<String>,
    gemini_model: &str,
    openai_key: &Option<String>,
    openai_base_url: &str,
    openai_model: &str,
    claude_key: &Option<String>,
    claude_model: &str,
) -> Result<Provider> {
    match name.to_lowercase().as_str() {
        "gemini" => {
            let key = gemini_key
                .clone()
                .context("Missing --gemini-key or GEMINI_API_KEY for provider gemini")?;
            Ok(Provider::Gemini {
                api_key: key,
                model: gemini_model.to_string(),
            })
        }
        "openai" => {
            let key = openai_key
                .clone()
                .context("Missing --openai-key or OPENAI_API_KEY for provider openai")?;
            Ok(Provider::OpenAI {
                api_key: key,
                base_url: openai_base_url.to_string(),
                model: openai_model.to_string(),
            })
        }
        "ollama" => Ok(Provider::OpenAI {
            api_key: String::new(),
            base_url: openai_base_url.to_string(),
            model: openai_model.to_string(),
        }),
        "claude" => {
            let key = claude_key
                .clone()
                .context("Missing --claude-key or ANTHROPIC_API_KEY for provider claude")?;
            Ok(Provider::Claude {
                api_key: key,
                model: claude_model.to_string(),
            })
        }
        other => bail!(
            "Unknown provider: '{}'. Supported: gemini, openai, ollama, claude",
            other
        ),
    }
}

pub async fn translate_page(
    input_path: &Path,
    output_path: &Path,
    yolo: &mut YoloModel,
    ctx: &TranslationContext<'_>,
) -> Result<()> {
    let jsonl = ctx.progress == "jsonl";
    let silent_events = ctx.events.is_some();
    let verbose = !(jsonl || ctx.quiet) && !silent_events;
    macro_rules! tinfo {
        ($($t:tt)*) => {
            if verbose { println!($($t)*) } else if !silent_events { eprintln!($($t)*) }
        };
    }
    let emit = |phase: &'static str| {
        if jsonl {
            eprintln!(
                "{}",
                serde_json::json!({
                    "idx": ctx.page_idx,
                    "total": ctx.page_total,
                    "page": input_path.file_name().unwrap_or_default().to_string_lossy(),
                    "phase": phase,
                })
            );
        }
        if let Some(on_event) = ctx.events {
            on_event(PageEvent {
                idx: ctx.page_idx,
                total: ctx.page_total,
                page: input_path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                phase,
                error: None,
            });
        }
    };
    let img =
        image::open(input_path).with_context(|| format!("Failed to open {:?}", input_path))?;
    let mut detections = yolo.detect_bubbles(&img)?;

    if detections.is_empty() && !ctx.translate_free_text {
        tinfo!("    [Page] No dialogue bubbles detected. Copying original.");
        img.save(output_path)?;
        return Ok(());
    }
    tinfo!("    [Page] Found {} bubbles.", detections.len());
    let orig_rgb = img.to_rgb8();
    let (w_img, h_img) = img.dimensions();

    // --- Freetext detection ---
    let mut ft_boxes: Vec<[u32; 4]> = Vec::new();
    if ctx.translate_free_text {
        if ctx.ocr == "none" {
            eprintln!("[freetext] butuh --ocr rapid|manga|tesseract, fallback bubble-only");
        } else {
            let script = crate::ocr::OcrScript::from_key(&ctx.ocr_script);
            let engine = crate::ocr::create_ocr_engine(&ctx.ocr, ctx.ocr_model.as_deref(), script);
            if engine.name() == "none" {
                eprintln!("[freetext] engine none, skip freetext");
            } else {
                let bubble_boxes: Vec<[u32; 4]> = detections
                    .iter()
                    .map(|d| [d.x1, d.y1, d.x2, d.y2])
                    .collect();
                let detected =
                    crate::preparer::detect_free_text(&orig_rgb, &bubble_boxes, engine.as_ref());
                if detected.is_empty() {
                    tinfo!("    [Freetext] 0 regions");
                } else {
                    tinfo!("    [Freetext] {} regions: {:?}", detected.len(), detected);
                    ft_boxes = detected;
                }
            }
        }
    }

    // Build combined detections for rendering/inpaint: bubbles + freetext
    let mut combined_dets: Vec<crate::model::yolo::Detection> = detections.clone();
    let mut ft_ids: Vec<String> = Vec::new();
    for (idx, fb) in ft_boxes.iter().enumerate() {
        let fid = format!("ft{}", idx + 1);
        ft_ids.push(fid);
        combined_dets.push(crate::model::yolo::Detection {
            x1: fb[0],
            y1: fb[1],
            x2: fb[2],
            y2: fb[3],
            conf: 0.90,
        });
    }
    if detections.is_empty() && combined_dets.is_empty() {
        tinfo!("    [Page] No bubbles nor freetext, copying original.");
        img.save(output_path)?;
        return Ok(());
    }
    emit("detect_end");

    // Build crops: bubbles + ft
    let mut crops: Vec<CropItem> = Vec::new();
    for (i, det) in detections.iter().enumerate() {
        let w = det.width();
        let h = det.height();
        if w == 0 || h == 0 {
            continue;
        }
        tinfo!(
            "      Bubble #{}: [{}, {}, {}, {}] ({}x{})",
            i + 1,
            det.x1,
            det.y1,
            det.x2,
            det.y2,
            w,
            h
        );
        let mut crop = RgbImage::new(w, h);
        for cy in 0..h {
            for cx in 0..w {
                crop.put_pixel(cx, cy, *orig_rgb.get_pixel(det.x1 + cx, det.y1 + cy));
            }
        }
        crops.push(CropItem {
            id: (i + 1).to_string(),
            image: crop,
        });
    }
    for (i, fb) in ft_boxes.iter().enumerate() {
        let pad = crate::preparer::freetext_pad(*fb, w_img, h_img);
        let w = (pad[2] - pad[0]).max(1);
        let h = (pad[3] - pad[1]).max(1);
        tinfo!(
            "      Freetext #{}: {:?} padded {:?} ({}x{})",
            i + 1,
            fb,
            pad,
            w,
            h
        );
        let mut crop = RgbImage::new(w, h);
        for cy in 0..h {
            for cx in 0..w {
                crop.put_pixel(cx, cy, *orig_rgb.get_pixel(pad[0] + cx, pad[1] + cy));
            }
        }
        crops.push(CropItem {
            id: format!("ft{}", i + 1),
            image: crop,
        });
    }

    // --- OCR raw_text gathering (for metadata) ---
    let mut raw_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if ctx.ocr != "none" {
        let script = crate::ocr::OcrScript::from_key(&ctx.ocr_script);
        let engine = crate::ocr::create_ocr_engine(&ctx.ocr, ctx.ocr_model.as_deref(), script);
        if engine.name() != "none" {
            // recognize per crop (bubble + ft)
            for c in &crops {
                // find bbox for this id
                let bbox_opt = if c.id.starts_with("ft") {
                    ft_boxes
                        .get(c.id[2..].parse::<usize>().unwrap_or(1) - 1)
                        .copied()
                } else {
                    c.id.parse::<usize>()
                        .ok()
                        .and_then(|idx| detections.get(idx - 1))
                        .map(|d| [d.x1, d.y1, d.x2, d.y2])
                };
                if let Some(bbox) = bbox_opt {
                    if let Some(txt) = engine.recognize(&orig_rgb, bbox) {
                        if !txt.trim().is_empty() {
                            raw_map.insert(c.id.clone(), txt);
                        }
                    }
                }
            }
            if !raw_map.is_empty() {
                tinfo!("    [OCR] raw_text {} entries", raw_map.len());
            }
        }
    }

    // --- Translation branching ---
    let mut all_translations: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    // Cache filter first
    let crops_to_translate: Vec<CropItem>;
    if let Some(cache) = ctx.cache {
        let prov_name = ctx.chain.primary.name();
        let model_name = ctx.chain.primary.model_name();
        let (cached, to_trans) = cache.filter_cached(
            &crops,
            ctx.target_lang,
            prov_name,
            model_name,
            ctx.prompt_sig,
        );
        if !cached.is_empty() {
            tinfo!(
                "      [Cache Hit] {}/{} from cache",
                cached.len(),
                crops.len()
            );
        }
        all_translations.extend(cached);
        crops_to_translate = to_trans;
    } else {
        crops_to_translate = crops.clone();
    }

    // Normalize ft ids to lowercase (LLM may return FT1)
    fn norm_map(
        mut m: std::collections::HashMap<String, String>,
    ) -> std::collections::HashMap<String, String> {
        let mut out = std::collections::HashMap::new();
        for (k, v) in m.drain() {
            let nk = if k.to_lowercase().starts_with("ft") {
                k.to_lowercase()
            } else {
                k
            };
            out.insert(nk, v);
        }
        out
    }
    // Helper for text-only translation
    async fn translate_ocr_json(
        chain: &ProviderChain,
        limiter: &RateLimiter,
        json_input: &str,
        prompt: &str,
        target_lang: &str,
        verbose: bool,
        silent: bool,
    ) -> Result<std::collections::HashMap<String, String>> {
        let full_prompt = format!(
            "{}\n\nInput JSON (id -> raw Japanese text):\n{}\n\nTranslate each value to {} and return JSON mapping id->translation.",
            prompt, json_input, target_lang
        );
        for prov in chain.all_providers() {
            if verbose {
                println!(
                    "  Translating OCR JSON with {} ({})...",
                    prov.name(),
                    prov.model_name()
                );
            } else if !silent {
                eprintln!(
                    "  Translating OCR JSON with {} ({})...",
                    prov.name(),
                    prov.model_name()
                );
            }
            let raw_res = limiter
                .execute_with_retry(verbose, || prov.translate_text(json_input, &full_prompt))
                .await;
            match raw_res {
                Ok(raw) => {
                    if let Ok(map) = crate::translation::parse_translation_json(&raw) {
                        if !map.is_empty() {
                            return Ok(norm_map(map));
                        }
                    }
                    if verbose {
                        println!("  [Failover] {} unparseable OCR json", prov.name());
                    } else if !silent {
                        eprintln!("  [Failover] {} unparseable OCR json", prov.name());
                    }
                }
                Err(e) => {
                    if verbose {
                        println!("  [Failover] {} failed ocr json: {}", prov.name(), e);
                    } else if !silent {
                        eprintln!("  [Failover] {} failed ocr json: {}", prov.name(), e);
                    }
                }
            }
        }
        bail!("All providers failed OCR JSON translation")
    }

    if crops_to_translate.is_empty() {
        tinfo!("      All served from cache");
    } else {
        let use_ocr_path = ctx.mode == "ocr" || ctx.mode == "auto";
        let ocr_available = !raw_map.is_empty() || ctx.ocr != "none";
        if ctx.mode == "ocr" && !ocr_available {
            if !silent_events {
                eprintln!("[mode ocr] no OCR results, fallback vision");
            }
        }
        if use_ocr_path && ctx.mode == "ocr" && ocr_available {
            // try OCR text-only
            // Build json for to_translate subset filtered by raw_map
            let mut ocr_json_map = serde_json::Map::new();
            for c in &crops_to_translate {
                if let Some(raw) = raw_map.get(&c.id) {
                    ocr_json_map.insert(c.id.clone(), serde_json::Value::String(raw.clone()));
                }
            }
            if ocr_json_map.is_empty() {
                // fallback vision if no raw
                for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
                    let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                    let chunk_trans = translate_with_chain(
                        ctx.chain,
                        &mosaic,
                        ctx.prompt,
                        ctx.target_lang,
                        ctx.rate_limiter,
                        verbose,
                    )
                    .await?;
                    if let Some(cache) = ctx.cache {
                        let pn = ctx.chain.primary.name();
                        let mn = ctx.chain.primary.model_name();
                        cache.save_batch(
                            &chunk_trans,
                            chunk,
                            ctx.target_lang,
                            pn,
                            mn,
                            ctx.prompt_sig,
                        );
                    }
                    all_translations.extend(chunk_trans);
                }
            } else {
                let json_str = serde_json::Value::Object(ocr_json_map).to_string();
                match translate_ocr_json(
                    ctx.chain,
                    ctx.rate_limiter,
                    &json_str,
                    ctx.prompt,
                    ctx.target_lang,
                    verbose,
                    silent_events,
                )
                .await
                {
                    Ok(map) => {
                        if let Some(cache) = ctx.cache {
                            let pn = ctx.chain.primary.name();
                            let mn = ctx.chain.primary.model_name();
                            cache.save_batch(
                                &map,
                                &crops_to_translate,
                                ctx.target_lang,
                                pn,
                                mn,
                                ctx.prompt_sig,
                            );
                        }
                        all_translations.extend(map);
                    }
                    Err(e) => {
                        if !silent_events {
                            eprintln!("[OCR] text translation failed: {}, fallback vision", e);
                        }
                        for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
                            let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                            let chunk_trans = translate_with_chain(
                                ctx.chain,
                                &mosaic,
                                ctx.prompt,
                                ctx.target_lang,
                                ctx.rate_limiter,
                                verbose,
                            )
                            .await?;
                            if let Some(cache) = ctx.cache {
                                let pn = ctx.chain.primary.name();
                                let mn = ctx.chain.primary.model_name();
                                cache.save_batch(
                                    &chunk_trans,
                                    chunk,
                                    ctx.target_lang,
                                    pn,
                                    mn,
                                    ctx.prompt_sig,
                                );
                            }
                            all_translations.extend(chunk_trans);
                        }
                    }
                }
            }
        } else if ctx.mode == "auto" && ocr_available && !raw_map.is_empty() {
            // auto: try ocr first, fallback vision on failure
            let mut ocr_json_map = serde_json::Map::new();
            for c in &crops_to_translate {
                if let Some(raw) = raw_map.get(&c.id) {
                    ocr_json_map.insert(c.id.clone(), serde_json::Value::String(raw.clone()));
                }
            }
            if ocr_json_map.is_empty() {
                for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
                    let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                    let chunk_trans = translate_with_chain(
                        ctx.chain,
                        &mosaic,
                        ctx.prompt,
                        ctx.target_lang,
                        ctx.rate_limiter,
                        verbose,
                    )
                    .await?;
                    if let Some(cache) = ctx.cache {
                        let pn = ctx.chain.primary.name();
                        let mn = ctx.chain.primary.model_name();
                        cache.save_batch(
                            &chunk_trans,
                            chunk,
                            ctx.target_lang,
                            pn,
                            mn,
                            ctx.prompt_sig,
                        );
                    }
                    all_translations.extend(chunk_trans);
                }
            } else {
                let json_str = serde_json::Value::Object(ocr_json_map).to_string();
                match translate_ocr_json(
                    ctx.chain,
                    ctx.rate_limiter,
                    &json_str,
                    ctx.prompt,
                    ctx.target_lang,
                    verbose,
                    silent_events,
                )
                .await
                {
                    Ok(map) => {
                        if let Some(cache) = ctx.cache {
                            let pn = ctx.chain.primary.name();
                            let mn = ctx.chain.primary.model_name();
                            cache.save_batch(
                                &map,
                                &crops_to_translate,
                                ctx.target_lang,
                                pn,
                                mn,
                                ctx.prompt_sig,
                            );
                        }
                        all_translations.extend(map);
                        // fill missing ids via vision
                        let missing: Vec<CropItem> = crops_to_translate
                            .iter()
                            .filter(|c| !all_translations.contains_key(&c.id))
                            .cloned()
                            .collect();
                        if !missing.is_empty() {
                            for chunk in missing.chunks(ctx.batch_size.max(1)) {
                                let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                                let chunk_trans = translate_with_chain(
                                    ctx.chain,
                                    &mosaic,
                                    ctx.prompt,
                                    ctx.target_lang,
                                    ctx.rate_limiter,
                                    verbose,
                                )
                                .await?;
                                all_translations.extend(chunk_trans);
                            }
                        }
                    }
                    Err(_) => {
                        for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
                            let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                            let chunk_trans = translate_with_chain(
                                ctx.chain,
                                &mosaic,
                                ctx.prompt,
                                ctx.target_lang,
                                ctx.rate_limiter,
                                verbose,
                            )
                            .await?;
                            if let Some(cache) = ctx.cache {
                                let pn = ctx.chain.primary.name();
                                let mn = ctx.chain.primary.model_name();
                                cache.save_batch(
                                    &chunk_trans,
                                    chunk,
                                    ctx.target_lang,
                                    pn,
                                    mn,
                                    ctx.prompt_sig,
                                );
                            }
                            all_translations.extend(chunk_trans);
                        }
                    }
                }
            }
        } else {
            // vision
            for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
                let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                let chunk_trans = translate_with_chain(
                    ctx.chain,
                    &mosaic,
                    ctx.prompt,
                    ctx.target_lang,
                    ctx.rate_limiter,
                    verbose,
                )
                .await?;
                if let Some(cache) = ctx.cache {
                    let pn = ctx.chain.primary.name();
                    let mn = ctx.chain.primary.model_name();
                    cache.save_batch(&chunk_trans, chunk, ctx.target_lang, pn, mn, ctx.prompt_sig);
                }
                all_translations.extend(chunk_trans);
            }
        }
    }

    // normalize ft keys to lowercase
    all_translations = norm_map(all_translations);
    // Glossary enforcement (single rewrite retry per leaked term).
    if let Some(g) = &ctx.glossary {
        if !g.is_empty() && !all_translations.is_empty() {
            let (h, m) = crate::translation::enforce_glossary(
                ctx.chain,
                ctx.rate_limiter,
                &mut all_translations,
                g,
                ctx.target_lang,
                verbose,
            )
            .await;
            ctx.gloss_hits.fetch_add(h, Ordering::SeqCst);
            ctx.gloss_misses.fetch_add(m, Ordering::SeqCst);
            if m > 0 || h > 0 {
                tinfo!("      [Glossary] hits={} misses={}", h, m);
            }
        }
    }
    emit("translate_end");
    for (k, v) in &all_translations {
        tinfo!("      #{} -> \"{}\"", k, v);
    }

    // Inpaint + typeset for combined (bubbles + ft)
    let mut page = orig_rgb.clone();
    let inpaint_targets: Vec<crate::model::yolo::Detection> = combined_dets
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let id = if *i < detections.len() {
                (i + 1).to_string()
            } else {
                format!("ft{}", i - detections.len() + 1)
            };
            if let Some(t) = all_translations.get(&id) {
                t.to_uppercase() != "SKIP" && !t.trim().is_empty()
            } else {
                false
            }
        })
        .map(|(_, d)| d.clone())
        .collect();
    if !inpaint_targets.is_empty() {
        inpaint_image(&mut page, &inpaint_targets)?;
    }
    let typesetter = Typesetter::new(ctx.font_bytes, ctx.cjk_font_bytes)?;
    for (idx, det) in combined_dets.iter().enumerate() {
        let id = if idx < detections.len() {
            (idx + 1).to_string()
        } else {
            format!("ft{}", idx - detections.len() + 1)
        };
        if let Some(text) = all_translations.get(&id) {
            if text.to_uppercase() == "SKIP" || text.trim().is_empty() {
                continue;
            }
            typesetter.render_bubble_text_with_style(
                &mut page,
                det,
                text,
                Some(ctx.target_lang),
                None,
                None,
            );
        }
    }
    page.save(output_path)?;
    emit("render_end");

    if ctx.save_metadata {
        let page_name = input_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let mut bubbles: Vec<metadata::Bubble> = Vec::new();
        for (i, det) in detections.iter().enumerate() {
            let id = (i + 1).to_string();
            bubbles.push(metadata::Bubble {
                id: id.clone(),
                bbox: [det.x1, det.y1, det.x2, det.y2],
                conf: det.conf,
                translated: all_translations.get(&id).cloned().unwrap_or_default(),
                bg_color: None,
                style: None,
                edited: false,
                raw_text: raw_map.get(&id).cloned(),
                mask_path: None,
            });
        }
        for (i, fb) in ft_boxes.iter().enumerate() {
            let id = format!("ft{}", i + 1);
            // sample bg median not critical
            bubbles.push(metadata::Bubble {
                id: id.clone(),
                bbox: *fb,
                conf: 0.90,
                translated: all_translations.get(&id).cloned().unwrap_or_default(),
                bg_color: None,
                style: None,
                edited: false,
                raw_text: raw_map.get(&id).cloned(),
                mask_path: None,
            });
        }
        let mut data = metadata::PageEditData::new(
            page_name.clone(),
            w_img,
            h_img,
            ctx.target_lang.to_string(),
            ctx.prompt_sig.to_string(),
            bubbles,
        );
        if ctx.ocr != "none" {
            data.ocr_engine = Some(ctx.ocr.clone());
        }
        // Initial reading order (manga R2L) so editors open with sane order.
        {
            let boxes: Vec<[u32; 4]> = data.bubbles.iter().map(|b| b.bbox).collect();
            let ord =
                crate::preparer::detect_reading_order(&boxes, crate::preparer::ReadingMode::R2L);
            if ord.len() == data.bubbles.len() {
                data.order = Some(ord.iter().map(|&i| data.bubbles[i].id.clone()).collect());
            }
        }
        let meta_dir = ctx
            .metadata_dir
            .clone()
            .unwrap_or_else(|| output_path.parent().unwrap_or(Path::new(".")).to_path_buf());
        std::fs::create_dir_all(&meta_dir)?;
        let meta_path = meta_dir.join(format!(
            "{}.kedit.json",
            output_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
        ));
        metadata::save_page_metadata(&meta_path, &data)?;
        tinfo!("    [Metadata] Saved to {:?}", meta_path);
    }
    Ok(())
}
