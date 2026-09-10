use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use kzktdk::archive::{self, PreparedInput};
use kzktdk::cache::{TranslationCache, prompt_signature};
use kzktdk::metadata;
use kzktdk::model::yolo::YoloModel;
use kzktdk::pipeline::{TranslationContext, build_provider, translate_page};
use kzktdk::translation::{ProviderChain, RateLimiter};

use super::util::{ensure_model, find_file_in_candidates, parse_jobs};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    export: String,
    model: PathBuf,
    target_lang: String,
    prompt: Option<String>,
    batch_size: usize,
    provider: String,
    fallback_provider: Option<String>,
    rate_limit: u32,
    jobs: String,
    no_cache: bool,
    clear_cache: bool,
    gemini_key: Option<String>,
    openai_key: Option<String>,
    openai_base_url: String,
    openai_model: String,
    gemini_model: String,
    claude_key: Option<String>,
    claude_model: String,
    font: PathBuf,
    cjk_font: PathBuf,
    save_metadata: bool,
    metadata_dir: Option<PathBuf>,
    ocr: String,
    translate_free_text: bool,
    ocr_script: String,
    mode: String,
    ocr_model: Option<PathBuf>,
    glossary: Option<PathBuf>,
    progress: String,
    format: String,
    quiet: bool,
    retry_failed: Option<PathBuf>,
) -> Result<()> {
    let jsonl = progress == "jsonl";
    let as_json = format == "json";
    let verbose = !(jsonl || quiet);
    macro_rules! tinfo {
        ($($t:tt)*) => {
            if verbose { println!($($t)*) } else { eprintln!($($t)*) }
        };
    }
    if clear_cache {
        let cache = TranslationCache::open()?;
        cache.clear()?;
        println!("[Cache] Cleared translation cache");
        return Ok(());
    }
    // Validate mode/ocr_script early
    let ocr_script_enum = kzktdk::ocr::OcrScript::from_key(&ocr_script);
    let _ = mode.clone();
    let _ = ocr_script_enum;
    let _ = ocr_model.clone();
    if translate_free_text && ocr == "none" {
        eprintln!("[freetext] butuh --ocr rapid|manga|tesseract, fallback bubble-only");
    }
    // Legacy info for mode
    if mode != "vision" && mode != "ocr" && mode != "auto" {
        eprintln!("[mode] unknown '{}', fallback vision", mode);
    }
    let input = input.context("Missing <INPUT> path (required unless --clear-cache)")?;

    let model_file = ensure_model(&model)?;

    tinfo!("==> Step 1: Initializing Pipeline & Model");
    // Resolve font via registry/config (global + per-bubble support)
    let font_bytes = {
        let font_str = font.to_string_lossy().to_string();
        // If font is a registry name (not a file), try resolve
        if !font.exists() && !font_str.contains('/') && !font_str.contains('\\') {
            if kzktdk::font::FontRegistry::resolve(&font_str).is_ok() {
                // Read from registry path
                let list = kzktdk::font::FontRegistry::list();
                if let Some(info) = list.iter().find(|f| f.name == font_str) {
                    if let Ok(b) = std::fs::read(&info.path) {
                        b
                    } else {
                        include_bytes!("../../fonts/Komika Axis.ttf").to_vec()
                    }
                } else {
                    include_bytes!("../../fonts/Komika Axis.ttf").to_vec()
                }
            } else if let Some(def) = kzktdk::font::AppConfig::get_latin() {
                if kzktdk::font::FontRegistry::resolve(&def).is_ok() {
                    let list = kzktdk::font::FontRegistry::list();
                    if let Some(info) = list.iter().find(|f| f.name == def) {
                        if let Ok(b) = std::fs::read(&info.path) {
                            b
                        } else {
                            include_bytes!("../../fonts/Komika Axis.ttf").to_vec()
                        }
                    } else {
                        include_bytes!("../../fonts/Komika Axis.ttf").to_vec()
                    }
                } else {
                    include_bytes!("../../fonts/Komika Axis.ttf").to_vec()
                }
            } else if let Some(p) = find_file_in_candidates("fonts/Komika Axis.ttf") {
                std::fs::read(p)?
            } else {
                include_bytes!("../../fonts/Komika Axis.ttf").to_vec()
            }
        } else if font.exists() {
            std::fs::read(&font)?
        } else if let Some(p) = find_file_in_candidates("fonts/Komika Axis.ttf") {
            std::fs::read(p)?
        } else {
            include_bytes!("../../fonts/Komika Axis.ttf").to_vec()
        }
    };

    let cjk_font_bytes = if cjk_font.exists() {
        std::fs::read(&cjk_font).ok()
    } else {
        let cjk_str = cjk_font.to_string_lossy().to_string();
        if !cjk_str.is_empty()
            && !cjk_str.contains('/')
            && !cjk_str.contains('\\')
            && kzktdk::font::FontRegistry::resolve(&cjk_str).is_ok()
        {
            let list = kzktdk::font::FontRegistry::list();
            if let Some(info) = list.iter().find(|f| f.name == cjk_str) {
                std::fs::read(&info.path).ok()
            } else {
                None
            }
        } else if let Some(def) = kzktdk::font::AppConfig::get_cjk() {
            let list = kzktdk::font::FontRegistry::list();
            if let Some(info) = list.iter().find(|f| f.name == def) {
                std::fs::read(&info.path).ok()
            } else if let Some(p) = find_file_in_candidates("fonts/KosugiMaru.ttf") {
                std::fs::read(p).ok()
            } else {
                None
            }
        } else if let Some(p) = find_file_in_candidates("fonts/KosugiMaru.ttf") {
            std::fs::read(p).ok()
        } else {
            None
        }
    };

    let primary = build_provider(
        &provider,
        &gemini_key,
        &gemini_model,
        &openai_key,
        &openai_base_url,
        &openai_model,
        &claude_key,
        &claude_model,
    )?;
    let mut fallbacks = Vec::new();
    if let Some(ref fb_str) = fallback_provider {
        for fb_name in fb_str
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            // avoid duplicate primary
            if fb_name.eq_ignore_ascii_case(&provider) {
                continue;
            }
            match build_provider(
                fb_name,
                &gemini_key,
                &gemini_model,
                &openai_key,
                &openai_base_url,
                &openai_model,
                &claude_key,
                &claude_model,
            ) {
                Ok(p) => fallbacks.push(p),
                Err(e) => eprintln!("  [Warn] Skipping fallback '{}': {}", fb_name, e),
            }
        }
    }
    let provider_chain = ProviderChain { primary, fallbacks };
    let rate_limiter = RateLimiter::new(rate_limit);
    let jobs_num = parse_jobs(&jobs);
    tinfo!(
        "  Jobs: {} (batch parallel), RateLimit: {} RPS",
        jobs_num,
        rate_limit
    );
    if !provider_chain.fallbacks.is_empty() {
        let fb_names: Vec<_> = provider_chain.fallbacks.iter().map(|p| p.name()).collect();
        tinfo!("  Fallbacks: {}", fb_names.join(", "));
    }

    // Glossary (exit 2 on invalid data).
    let glossary_map: Option<BTreeMap<String, String>> = if let Some(ref gpath) = glossary {
        match kzktdk::translation::load_glossary(gpath) {
            Ok(g) => {
                tinfo!("  Glossary: {} terms from {:?}", g.len(), gpath);
                Some(g)
            }
            Err(e) => {
                eprintln!("Invalid glossary: {:#}", e);
                std::process::exit(2);
            }
        }
    } else {
        None
    };
    let gloss_hits = Arc::new(AtomicUsize::new(0));
    let gloss_misses = Arc::new(AtomicUsize::new(0));

    // Ctrl-C: finish in-flight pages, skip the rest, exit 130.
    tokio::spawn(async {
        let _ = tokio::signal::ctrl_c().await;
        CANCELLED.store(true, Ordering::SeqCst);
        eprintln!("[cancel] Ctrl-C received, finishing in-flight pages...");
    });

    let use_cache = !no_cache;
    let prompt_sig = prompt_signature(prompt.as_deref());
    let cache_opt = if use_cache {
        match TranslationCache::open() {
            Ok(c) => {
                tinfo!("  Cache: enabled (prompt_sig={})", prompt_sig);
                Some(Arc::new(c))
            }
            Err(e) => {
                eprintln!("  [Warn] Cache disabled: {}", e);
                None
            }
        }
    } else {
        tinfo!("  Cache: disabled");
        None
    };

    let mut final_prompt = kzktdk::translation::build_translation_prompt_with_glossary(
        &target_lang,
        glossary_map.as_ref(),
    );
    if let Some(ref custom) = prompt {
        final_prompt.push_str("\n\nADDITIONAL TRANSLATION RULES:\n");
        final_prompt.push_str(custom);
    }

    // Single image: one YOLO load. Batch parallel below uses a
    // per-worker model pool instead of one load per page.
    tinfo!("==> Step 2: Preparing Input ({:?})", input);
    let prepared = archive::prepare_input(&input)?;

    match prepared {
        PreparedInput::SingleImage(img_path) => {
            let mut yolo = YoloModel::new(&model_file)?;
            let out_path = output.clone().unwrap_or_else(|| {
                let stem = img_path.file_stem().unwrap_or_default().to_string_lossy();
                let ext = img_path.extension().unwrap_or_default().to_string_lossy();
                img_path
                    .parent()
                    .unwrap_or(Path::new("."))
                    .join(format!("{}_translated.{}", stem, ext))
            });

            let ctx = TranslationContext {
                events: None,
                chain: &provider_chain,
                rate_limiter: &rate_limiter,
                prompt: &final_prompt,
                prompt_sig: &prompt_sig,
                target_lang: &target_lang,
                font_bytes: &font_bytes,
                cjk_font_bytes: cjk_font_bytes.as_deref(),
                batch_size,
                cache: cache_opt.as_deref(),
                save_metadata,
                metadata_dir: metadata_dir.clone(),
                translate_free_text: translate_free_text.clone(),
                ocr: ocr.clone(),
                ocr_script: ocr_script.clone(),
                mode: mode.clone(),
                ocr_model: ocr_model.clone(),
                glossary: glossary_map.clone(),
                gloss_hits: gloss_hits.clone(),
                gloss_misses: gloss_misses.clone(),
                progress: progress.clone(),
                quiet,
                page_idx: 1,
                page_total: 1,
            };
            let fname = img_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            // --retry-failed: reuse previous good output when available.
            let mut rec = PageRecord {
                idx: 0,
                file: fname,
                out: out_path.to_string_lossy().to_string(),
                ok: true,
                skipped: false,
                err: None,
            };
            if let Some(ref rdir) = retry_failed {
                if let Some(prev) = retry_hit(rdir, img_path.file_name().unwrap_or_default()) {
                    if let Some(parent) = out_path.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(&prev, &out_path)?;
                    rec.skipped = true;
                    tinfo!("    [Retry] Reusing previous output {:?}", prev);
                }
            }
            if !rec.skipped {
                if let Err(e) = translate_page(&img_path, &out_path, &mut yolo, &ctx).await {
                    rec.ok = false;
                    rec.err = Some(e.to_string());
                }
            }

            // Save project.kedit.json for single image if requested
            if save_metadata {
                let meta_dir = metadata_dir
                    .clone()
                    .unwrap_or_else(|| out_path.parent().unwrap_or(Path::new(".")).to_path_buf());
                let proj = PathBuf::from(&meta_dir).join("project.kedit.json");
                let sidecar = meta_dir.join(format!(
                    "{}.kedit.json",
                    out_path.file_stem().unwrap_or_default().to_string_lossy()
                ));
                metadata::save_project(&proj, &[sidecar], Some(target_lang.clone()))?;
                tinfo!("[Metadata] Project saved to {:?}", proj);
            }

            tinfo!("\n==> Success! Translated manga saved to {:?}", out_path);
            finish_translate(
                &[rec],
                gloss_hits.load(Ordering::SeqCst),
                gloss_misses.load(Ordering::SeqCst),
                out_path.to_string_lossy().to_string(),
                jsonl,
                as_json,
                verbose,
            );
        }

        PreparedInput::Batch {
            images,
            _temp_guard,
            is_archive,
            is_pdf,
            original_name,
        } => {
            tinfo!(
                "==> Found {} pages (natural order) to translate.",
                images.len()
            );

            let export_as_pdf = match export.to_lowercase().as_str() {
                "pdf" => true,
                "cbz" | "folder" => false,
                _ => {
                    if let Some(ref out) = output {
                        out.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("pdf"))
                            .unwrap_or(false)
                    } else {
                        is_pdf
                    }
                }
            };
            let export_as_cbz = match export.to_lowercase().as_str() {
                "cbz" => true,
                "folder" => false,
                _ => {
                    if let Some(ref out) = output {
                        out.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("cbz"))
                            .unwrap_or(false)
                    } else {
                        is_archive
                    }
                }
            };

            let temp_output_dir = if export_as_cbz || export_as_pdf {
                // Archive outputs (CBZ/PDF) are packed from page images at
                // the end; `-o` names the final file, not a folder.
                tempfile::tempdir()?.keep()
            } else if let Some(ref out) = output {
                out.clone()
            } else {
                let parent = input.parent().unwrap_or(Path::new("."));
                parent.join(format!("{}_translated", original_name))
            };

            std::fs::create_dir_all(&temp_output_dir)?;

            // Parallel batch translation
            if jobs_num <= 1 || images.len() <= 1 {
                // Sequential fallback
                let mut translated_files = Vec::new();
                let mut records: Vec<PageRecord> = Vec::new();
                let mut yolo = YoloModel::new(&model_file)?;
                let total_pages = images.len();
                let mut ctx = TranslationContext {
                    events: None,
                    chain: &provider_chain,
                    rate_limiter: &rate_limiter,
                    prompt: &final_prompt,
                    prompt_sig: &prompt_sig,
                    target_lang: &target_lang,
                    font_bytes: &font_bytes,
                    cjk_font_bytes: cjk_font_bytes.as_deref(),
                    batch_size,
                    cache: cache_opt.as_deref(),
                    save_metadata,
                    metadata_dir: metadata_dir.clone(),
                    translate_free_text: translate_free_text.clone(),
                    ocr: ocr.clone(),
                    ocr_script: ocr_script.clone(),
                    mode: mode.clone(),
                    ocr_model: ocr_model.clone(),
                    glossary: glossary_map.clone(),
                    gloss_hits: gloss_hits.clone(),
                    gloss_misses: gloss_misses.clone(),
                    progress: progress.clone(),
                    quiet,
                    page_idx: 1,
                    page_total: total_pages,
                };
                for (idx, file_path) in images.iter().enumerate() {
                    if CANCELLED.load(Ordering::SeqCst) {
                        for (rest_idx, rest) in images.iter().enumerate().skip(idx) {
                            records.push(PageRecord {
                                idx: rest_idx,
                                file: rest
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .to_string(),
                                out: String::new(),
                                ok: false,
                                skipped: false,
                                err: Some("cancelled".to_string()),
                            });
                        }
                        break;
                    }
                    ctx.page_idx = idx + 1;
                    let file_name = file_path.file_name().unwrap();
                    let target_path = temp_output_dir.join(file_name);
                    tinfo!(
                        "\n[Page {}/{}] Translating: {:?}",
                        idx + 1,
                        images.len(),
                        file_name
                    );
                    let mut rec = PageRecord {
                        idx,
                        file: file_name.to_string_lossy().to_string(),
                        out: target_path.to_string_lossy().to_string(),
                        ok: true,
                        skipped: false,
                        err: None,
                    };
                    if let Some(ref rdir) = retry_failed {
                        if let Some(prev) = retry_hit(rdir, file_name) {
                            let _ = std::fs::copy(&prev, &target_path);
                            rec.skipped = true;
                            tinfo!("    [Retry] Reusing previous output {:?}", prev);
                            translated_files.push(target_path);
                            records.push(rec);
                            continue;
                        }
                    }
                    if let Err(e) = translate_page(file_path, &target_path, &mut yolo, &ctx).await {
                        eprintln!(
                            "    [!] Error translating {:?}: {}. Copying original.",
                            file_name, e
                        );
                        let _ = std::fs::copy(file_path, &target_path);
                        rec.ok = false;
                        rec.err = Some(e.to_string());
                    } else {
                        tinfo!("    Saved -> {:?}", target_path);
                    }
                    translated_files.push(target_path);
                    records.push(rec);
                }
                if save_metadata {
                    let meta_dir = metadata_dir
                        .clone()
                        .unwrap_or_else(|| temp_output_dir.clone());
                    std::fs::create_dir_all(&meta_dir)?;
                    let sidecars: Vec<PathBuf> = translated_files
                        .iter()
                        .map(|p| {
                            meta_dir.join(format!(
                                "{}.kedit.json",
                                p.file_stem().unwrap_or_default().to_string_lossy()
                            ))
                        })
                        .collect();
                    let proj = meta_dir.join("project.kedit.json");
                    let _ = metadata::save_project(&proj, &sidecars, Some(target_lang.clone()));
                    tinfo!("[Metadata] Project saved to {:?}", proj);
                }
                let output_desc = if export_as_pdf {
                    let pdf_out = if let Some(ref out) = output {
                        out.clone()
                    } else {
                        let parent = input.parent().unwrap_or(Path::new("."));
                        parent.join(format!("{}_translated.pdf", original_name))
                    };
                    tinfo!(
                        "\n==> Packing {} pages into PDF: {:?}",
                        translated_files.len(),
                        pdf_out
                    );
                    archive::create_pdf(&translated_files, &pdf_out)?;
                    tinfo!("==> PDF Export Complete: {:?}", pdf_out);
                    pdf_out.to_string_lossy().to_string()
                } else if export_as_cbz {
                    let cbz_out = if let Some(ref out) = output {
                        out.clone()
                    } else {
                        let parent = input.parent().unwrap_or(Path::new("."));
                        parent.join(format!("{}_translated.cbz", original_name))
                    };
                    tinfo!(
                        "\n==> Packing {} pages into CBZ: {:?}",
                        translated_files.len(),
                        cbz_out
                    );
                    archive::create_cbz(&translated_files, &cbz_out)?;
                    tinfo!("==> CBZ Export Complete: {:?}", cbz_out);
                    cbz_out.to_string_lossy().to_string()
                } else {
                    tinfo!(
                        "\n==> All {} pages successfully saved to {:?}",
                        translated_files.len(),
                        temp_output_dir
                    );
                    temp_output_dir.to_string_lossy().to_string()
                };
                finish_translate(
                    &records,
                    gloss_hits.load(Ordering::SeqCst),
                    gloss_misses.load(Ordering::SeqCst),
                    output_desc,
                    jsonl,
                    as_json,
                    verbose,
                );
            } else {
                // Parallel with JoinSet + Semaphore
                use tokio::sync::Semaphore;
                let semaphore = Arc::new(Semaphore::new(jobs_num));
                let mut join_set = tokio::task::JoinSet::new();
                let completed = Arc::new(AtomicUsize::new(0));
                let total = images.len();
                let final_prompt_arc = Arc::new(final_prompt.clone());
                let target_lang_arc = Arc::new(target_lang.clone());
                let font_bytes_arc = Arc::new(font_bytes.clone());
                let cjk_opt_arc = Arc::new(cjk_font_bytes.clone());
                let prompt_sig_arc = Arc::new(prompt_sig.clone());
                // per-task chain rebuild from config strings
                let provider_name = provider.clone();
                let gemini_key_c = gemini_key.clone();
                let gemini_model_c = gemini_model.clone();
                let openai_key_c = openai_key.clone();
                let openai_base_c = openai_base_url.clone();
                let openai_model_c = openai_model.clone();
                let claude_key_c = claude_key.clone();
                let claude_model_c = claude_model.clone();
                let fallback_str_c = fallback_provider.clone();
                let rate_limit_c = rate_limit;
                let use_cache_flag = cache_opt.is_some();
                let save_meta_flag = save_metadata;
                let meta_dir_opt = metadata_dir.clone();
                let translate_free_text_c = translate_free_text.clone();
                let ocr_c = ocr.clone();
                let ocr_script_c = ocr_script.clone();
                let mode_c = mode.clone();
                let ocr_model_c = ocr_model.clone();
                let glossary_c = glossary_map.clone();
                let progress_c = progress.clone();
                let gloss_hits_c = gloss_hits.clone();
                let gloss_misses_c = gloss_misses.clone();
                let retry_c = retry_failed.clone();
                // YOLO model pool: one model per worker, shared across pages.
                // (YoloModel: Send — see send_tests; checkout is a short
                // std::Mutex pop/push, never held across await.)
                let pool_size = jobs_num.min(total).max(1);
                let mut pool_models = Vec::with_capacity(pool_size);
                for _ in 0..pool_size {
                    pool_models.push(YoloModel::new(&model_file)?);
                }
                let pool_sem = Arc::new(Semaphore::new(pool_size));
                let pool = Arc::new(std::sync::Mutex::new(pool_models));
                if verbose {
                    println!(
                        "[Batch] YOLO pool: {} model(s) shared across {} pages",
                        pool_size, total
                    );
                }

                for (idx, file_path) in images.iter().enumerate() {
                    let sem = semaphore.clone();
                    let pool_sem_c = pool_sem.clone();
                    let pool_c = pool.clone();
                    let out_dir = temp_output_dir.clone();
                    let file_path = file_path.clone();
                    let final_prompt = final_prompt_arc.clone();
                    let target_lang = target_lang_arc.clone();
                    let font_bytes = font_bytes_arc.clone();
                    let cjk_opt = cjk_opt_arc.clone();
                    let prompt_sig = prompt_sig_arc.clone();
                    let provider_name = provider_name.clone();
                    let gemini_key_c = gemini_key_c.clone();
                    let gemini_model_c = gemini_model_c.clone();
                    let openai_key_c = openai_key_c.clone();
                    let openai_base_c = openai_base_c.clone();
                    let openai_model_c = openai_model_c.clone();
                    let claude_key_c = claude_key_c.clone();
                    let claude_model_c = claude_model_c.clone();
                    let fallback_str_c = fallback_str_c.clone();
                    let save_meta_flag = save_meta_flag;
                    let meta_dir_opt = meta_dir_opt.clone();
                    let translate_free_text_c2 = translate_free_text_c.clone();
                    let ocr_c2 = ocr_c.clone();
                    let ocr_script_c2 = ocr_script_c.clone();
                    let mode_c2 = mode_c.clone();
                    let ocr_model_c2 = ocr_model_c.clone();
                    let glossary_c2 = glossary_c.clone();
                    let progress_c2 = progress_c.clone();
                    let gloss_hits_c2 = gloss_hits_c.clone();
                    let gloss_misses_c2 = gloss_misses_c.clone();
                    let retry_c2 = retry_c.clone();
                    let total_c = total;
                    join_set.spawn(async move {
                        let _permit = sem.acquire_owned().await.unwrap();
                        let file_name = file_path.file_name().unwrap().to_owned();
                        let target_path = out_dir.join(&file_name);
                        // --retry-failed: reuse previous good output without loading models.
                        if let Some(ref rdir) = retry_c2 {
                            if let Some(prev) = retry_hit(rdir, file_name.as_os_str()) {
                                let _ = std::fs::copy(&prev, &target_path);
                                return (target_path, idx, true, None, true);
                            }
                        }
                        // Rebuild chain per task (cheap)
                        let primary = build_provider(
                            &provider_name,
                            &gemini_key_c,
                            &gemini_model_c,
                            &openai_key_c,
                            &openai_base_c,
                            &openai_model_c,
                            &claude_key_c,
                            &claude_model_c,
                        )
                        .expect("primary provider build failed");
                        let mut fallbacks = Vec::new();
                        if let Some(fb) = fallback_str_c {
                            for fb_name in fb.split(',').map(|s| s.trim()).filter(|s| !s.is_empty())
                            {
                                if fb_name.eq_ignore_ascii_case(&provider_name) {
                                    continue;
                                }
                                if let Ok(p) = build_provider(
                                    fb_name,
                                    &gemini_key_c,
                                    &gemini_model_c,
                                    &openai_key_c,
                                    &openai_base_c,
                                    &openai_model_c,
                                    &claude_key_c,
                                    &claude_model_c,
                                ) {
                                    fallbacks.push(p);
                                }
                            }
                        }
                        let chain = ProviderChain { primary, fallbacks };
                        let limiter = RateLimiter::new(rate_limit_c);
                        // Checkout one pooled model (released below before returning).
                        let _pool_permit = pool_sem_c.acquire_owned().await.unwrap();
                        let mut yolo = pool_c.lock().unwrap().pop().expect("yolo pool exhausted");
                        // per-task cache (open fresh connection to avoid !Send)
                        let cache_local: Option<TranslationCache> = if use_cache_flag {
                            TranslationCache::open().ok()
                        } else {
                            None
                        };
                        let ctx = TranslationContext {
                            events: None,
                            chain: &chain,
                            rate_limiter: &limiter,
                            prompt: &final_prompt,
                            prompt_sig: &prompt_sig,
                            target_lang: &target_lang,
                            font_bytes: &font_bytes,
                            cjk_font_bytes: cjk_opt.as_ref().as_deref().map(|v| v as &[u8]),
                            batch_size,
                            cache: cache_local.as_ref(),
                            save_metadata: save_meta_flag,
                            metadata_dir: meta_dir_opt.clone(),
                            translate_free_text: translate_free_text_c2,
                            ocr: ocr_c2,
                            ocr_script: ocr_script_c2,
                            mode: mode_c2,
                            ocr_model: ocr_model_c2,
                            glossary: glossary_c2,
                            gloss_hits: gloss_hits_c2,
                            gloss_misses: gloss_misses_c2,
                            progress: progress_c2,
                            quiet,
                            page_idx: idx + 1,
                            page_total: total_c,
                        };
                        let res = translate_page(&file_path, &target_path, &mut yolo, &ctx).await;
                        pool_c.lock().unwrap().push(yolo);
                        match res {
                            Ok(()) => (target_path, idx, true, None, false),
                            Err(e) => {
                                let msg = e.to_string();
                                let _ = std::fs::copy(&file_path, &target_path);
                                (target_path, idx, false, Some(msg), false)
                            }
                        }
                    });
                }

                let mut results: Vec<(PathBuf, usize, bool, Option<String>, bool)> = Vec::new();
                let mut pending: std::collections::HashSet<usize> = (0..images.len()).collect();
                let mut cancel_logged = false;
                loop {
                    if CANCELLED.load(Ordering::SeqCst) && !cancel_logged {
                        cancel_logged = true;
                        eprintln!("[cancel] Ctrl-C: aborting queued pages, finishing in-flight...");
                        join_set.abort_all();
                    }
                    match join_set.join_next().await {
                        Some(Ok((p, idx, ok, err, skipped))) => {
                            pending.remove(&idx);
                            let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
                            if verbose {
                                if ok {
                                    println!(
                                        "[Page {}/{}] Done {:?} -> {:?}{}",
                                        done,
                                        total,
                                        images[idx].file_name().unwrap_or_default(),
                                        p,
                                        if skipped { " (reused)" } else { "" }
                                    );
                                } else {
                                    println!(
                                        "[Page {}/{}] [!] Failed {:?}: {}",
                                        done,
                                        total,
                                        images[idx].file_name().unwrap_or_default(),
                                        err.as_deref().unwrap_or("unknown")
                                    );
                                }
                            } else if !ok {
                                eprintln!(
                                    "[Page {}/{}] [!] Failed {:?}: {}",
                                    done,
                                    total,
                                    images[idx].file_name().unwrap_or_default(),
                                    err.as_deref().unwrap_or("unknown")
                                );
                            }
                            results.push((p, idx, ok, err, skipped));
                        }
                        Some(Err(_)) => {
                            // Aborted task; resolved via pending set below.
                        }
                        None => break,
                    }
                }
                results.sort_by_key(|(_, idx, _, _, _)| *idx);
                let mut records: Vec<PageRecord> = results
                    .into_iter()
                    .map(|(p, idx, ok, err, skipped)| PageRecord {
                        idx,
                        file: images[idx]
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string(),
                        out: p.to_string_lossy().to_string(),
                        ok,
                        skipped,
                        err,
                    })
                    .collect();
                for idx in pending {
                    if !records.iter().any(|r| r.idx == idx) {
                        records.push(PageRecord {
                            idx,
                            file: images[idx]
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            out: String::new(),
                            ok: false,
                            skipped: false,
                            err: Some("cancelled".to_string()),
                        });
                    }
                }
                records.sort_by_key(|r| r.idx);
                let translated_files: Vec<PathBuf> = records
                    .iter()
                    .filter(|r| !r.out.is_empty())
                    .map(|r| PathBuf::from(&r.out))
                    .collect();

                if save_meta_flag {
                    let meta_dir = meta_dir_opt
                        .clone()
                        .unwrap_or_else(|| temp_output_dir.clone());
                    std::fs::create_dir_all(&meta_dir)?;
                    let sidecars: Vec<PathBuf> = translated_files
                        .iter()
                        .map(|p| {
                            meta_dir.join(format!(
                                "{}.kedit.json",
                                p.file_stem().unwrap_or_default().to_string_lossy()
                            ))
                        })
                        .collect();
                    let proj = meta_dir.join("project.kedit.json");
                    let _ = metadata::save_project(&proj, &sidecars, Some(target_lang.clone()));
                    tinfo!("[Metadata] Project saved to {:?}", proj);
                }

                let output_desc = if export_as_pdf {
                    let pdf_out = if let Some(ref out) = output {
                        out.clone()
                    } else {
                        let parent = input.parent().unwrap_or(Path::new("."));
                        parent.join(format!("{}_translated.pdf", original_name))
                    };
                    tinfo!(
                        "\n==> Packing {} pages into PDF: {:?}",
                        translated_files.len(),
                        pdf_out
                    );
                    archive::create_pdf(&translated_files, &pdf_out)?;
                    tinfo!("==> PDF Export Complete: {:?}", pdf_out);
                    pdf_out.to_string_lossy().to_string()
                } else if export_as_cbz {
                    let cbz_out = if let Some(ref out) = output {
                        out.clone()
                    } else {
                        let parent = input.parent().unwrap_or(Path::new("."));
                        parent.join(format!("{}_translated.cbz", original_name))
                    };
                    tinfo!(
                        "\n==> Packing {} pages into CBZ: {:?}",
                        translated_files.len(),
                        cbz_out
                    );
                    archive::create_cbz(&translated_files, &cbz_out)?;
                    tinfo!("==> CBZ Export Complete: {:?}", cbz_out);
                    cbz_out.to_string_lossy().to_string()
                } else {
                    tinfo!(
                        "\n==> All {} pages successfully saved to {:?}",
                        translated_files.len(),
                        temp_output_dir
                    );
                    temp_output_dir.to_string_lossy().to_string()
                };
                finish_translate(
                    &records,
                    gloss_hits.load(Ordering::SeqCst),
                    gloss_misses.load(Ordering::SeqCst),
                    output_desc,
                    jsonl,
                    as_json,
                    verbose,
                );
            }
        }
    }
    Ok(())
}

static CANCELLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// One page result for the machine-readable translate summary.
#[derive(Debug, Clone)]
struct PageRecord {
    idx: usize,
    file: String,
    out: String,
    ok: bool,
    skipped: bool,
    err: Option<String>,
}

/// Checks a previous-run folder for a reusable output (`--retry-failed`).
/// A page counts as already-good when its output file exists AND (no sidecar
/// exists OR the sidecar has at least one real translation).
fn retry_hit(retry_dir: &Path, file_name: &std::ffi::OsStr) -> Option<PathBuf> {
    let prev = retry_dir.join(file_name);
    if !prev.is_file() {
        return None;
    }
    let stem = Path::new(file_name)
        .file_stem()?
        .to_string_lossy()
        .to_string();
    let sidecar = retry_dir.join(format!("{}.kedit.json", stem));
    if sidecar.is_file() {
        match metadata::load_page_metadata(&sidecar) {
            Ok(data) => {
                let any_text = data.bubbles.iter().any(|b| {
                    let t = b.translated.trim();
                    !t.is_empty() && t.to_uppercase() != "SKIP"
                });
                if !any_text {
                    return None;
                }
            }
            Err(_) => return None,
        }
    }
    Some(prev)
}

fn translate_summary(
    records: &[PageRecord],
    gloss_hits: usize,
    gloss_misses: usize,
    output: &str,
) -> serde_json::Value {
    let ok_count = records.iter().filter(|r| r.ok).count();
    serde_json::json!({
        "ok": records.iter().all(|r| r.ok),
        "files": records.iter().map(|r| serde_json::json!({
            "idx": r.idx, "file": r.file, "out": r.out,
            "ok": r.ok, "skipped": r.skipped, "error": r.err,
        })).collect::<Vec<_>>(),
        "ok_count": ok_count,
        "fail_count": records.len().saturating_sub(ok_count),
        "skipped_count": records.iter().filter(|r| r.skipped).count(),
        "glossary_hits": gloss_hits,
        "glossary_misses": gloss_misses,
        "output": output,
    })
}

/// Emits the final translate summary (text/jsonl/json) and exits 1 on failures.
fn finish_translate(
    records: &[PageRecord],
    gloss_hits: usize,
    gloss_misses: usize,
    output_desc: String,
    jsonl: bool,
    as_json: bool,
    verbose: bool,
) {
    let summary = translate_summary(records, gloss_hits, gloss_misses, &output_desc);
    let fails = records.iter().filter(|r| !r.ok).count();
    if jsonl {
        let mut m = summary.as_object().cloned().unwrap_or_default();
        m.insert("done".into(), serde_json::Value::Bool(true));
        eprintln!("{}", serde_json::Value::Object(m));
    } else if verbose {
        println!(
            "\n==> Translated {}/{} pages -> {}",
            records.len().saturating_sub(fails),
            records.len(),
            output_desc
        );
        if fails > 0 {
            println!("    [!] {} page(s) failed (original copied)", fails);
        }
    } else {
        eprintln!(
            "Translated {}/{} pages -> {}",
            records.len().saturating_sub(fails),
            records.len(),
            output_desc
        );
    }
    if as_json {
        println!("{}", serde_json::to_string_pretty(&summary).unwrap());
    }
    if CANCELLED.load(Ordering::SeqCst) {
        std::process::exit(130);
    }
    if fails > 0 {
        std::process::exit(1);
    }
}
