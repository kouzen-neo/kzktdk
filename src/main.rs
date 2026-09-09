use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use image::{Rgb, RgbImage};
use std::path::{Path, PathBuf};

use kzktdk::archive::{self, PreparedInput};
use kzktdk::inpaint::inpaint_image;
use kzktdk::model::decrypt::decrypt_model;
use kzktdk::model::yolo::YoloModel;
use kzktdk::translation::{CropItem, MosaicBuilder, Provider, build_translation_prompt};
use kzktdk::typesetting::Typesetter;

const MAIN_HELP_TEMPLATE: &str = "\
{name} <command> [options]

Usage:

  kzktdk translate <archive.cbz>                 Translate comic archive (.cbz / .zip)
  kzktdk translate <archive.cbz> -t Indonesian   Translate to specific target language
  kzktdk translate <folder/> --export folder     Translate image directory in batch
  kzktdk translate <folder/> -o <chapter.cbz>    Translate folder directly into CBZ
  kzktdk translate <page.jpg> -o <out.jpg>       Translate single comic page
  kzktdk translate <file> --provider ollama      Offline translation with local vision model
  kzktdk translate -h                            Quick help on translation options

Commands:

  translate   Full pipeline: Detect -> Inpaint -> Translate -> Typeset
  help        Print this message or the help of the given subcommand(s)

Specify API keys via environment variables:
  GEMINI_API_KEY      Google Gemini API key (recommended)
  OPENAI_API_KEY      OpenAI / OpenRouter API key
  ANTHROPIC_API_KEY   Anthropic Claude API key
or on the command line via: --gemini-key, --openai-key, --claude-key

Options:
{options}

kzktdk@{version}
";

#[derive(Parser)]
#[command(
    name = "kzktdk",
    version = "0.1.0",
    about = "KZKT Desktop - High-Performance Comic & Manga Translation CLI",
    help_template = MAIN_HELP_TEMPLATE
)]
struct Cli {
    /// Hidden developer reference manual
    #[arg(long, hide = true)]
    dev_help: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Commands {
    /// Decrypt the bundled/custom model (.dat -> .onnx)
    #[command(hide = true)]
    DecryptModel {
        #[arg(short, long, default_value = "models/kzkt.dat")]
        source: PathBuf,
        #[arg(short, long, default_value = "models/kzkt.onnx")]
        dest: PathBuf,
    },

    /// Detect speech bubbles and draw bounding boxes onto output image
    #[command(hide = true)]
    Detect {
        /// Input image path
        input: PathBuf,
        /// Output annotated image path
        #[arg(short, long, default_value = "detected.png")]
        output: PathBuf,
        /// ONNX model path (will auto-decrypt models/kzkt.dat if missing)
        #[arg(short, long, default_value = "models/kzkt.onnx")]
        model: PathBuf,
    },

    /// Inpaint/erase original text inside dialogue bubbles
    #[command(hide = true)]
    Inpaint {
        /// Input image path
        input: PathBuf,
        /// Output inpainted image path
        #[arg(short, long, default_value = "inpainted.png")]
        output: PathBuf,
        /// ONNX model path
        #[arg(short, long, default_value = "models/kzkt.onnx")]
        model: PathBuf,
    },

    /// Full pipeline: Detect -> Inpaint -> Translate -> Typeset
    #[command(after_help = "\
Examples:
  kzktdk translate \"chapter_01.cbz\"
  kzktdk translate \"chapter_01.cbz\" -t Indonesian --prompt \"Gaya bahasa santai\"
  kzktdk translate \"./raw_pages/\" --export cbz -o \"chapter_01_id.cbz\"
  kzktdk translate \"page_01.jpg\" -o \"page_01_translated.jpg\"
  kzktdk translate \"chapter_01.cbz\" --provider ollama --openai-base-url \"http://localhost:11434/v1\" --openai-model \"llama3.2-vision\"")]
    Translate {
        /// Input path: image (.jpg/.png/.webp), folder, or comic archive (.cbz/.zip)
        input: PathBuf,
        /// Output path (image file, folder, or .cbz file)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Export format: 'folder', 'cbz', or 'auto' (defaults to 'auto')
        #[arg(long, default_value = "auto")]
        export: String,
        /// ONNX model path
        #[arg(short, long, default_value = "models/kzkt.onnx")]
        model: PathBuf,
        /// Target translation language
        #[arg(short, long, default_value = "English")]
        target_lang: String,
        /// Custom prompt instructions or additional translation rules
        #[arg(long)]
        prompt: Option<String>,
        /// Number of dialogue bubbles to batch per LLM translation request
        #[arg(long, default_value = "15")]
        batch_size: usize,
        /// LLM Provider: gemini, openai, ollama, or claude
        #[arg(short, long, default_value = "gemini")]
        provider: String,
        /// Gemini API Key (or set GEMINI_API_KEY env var)
        #[arg(long, env = "GEMINI_API_KEY")]
        gemini_key: Option<String>,
        /// OpenAI API Key (or set OPENAI_API_KEY env var)
        #[arg(long, env = "OPENAI_API_KEY")]
        openai_key: Option<String>,
        /// OpenAI base URL (use for Ollama e.g. http://localhost:11434/v1)
        #[arg(long, default_value = "https://api.openai.com/v1")]
        openai_base_url: String,
        /// Model name for OpenAI / Ollama
        #[arg(long, default_value = "gpt-4o-mini")]
        openai_model: String,
        /// Model name for Gemini
        #[arg(long, default_value = "gemini-3.1-flash-lite")]
        gemini_model: String,
        /// Claude API Key (or set ANTHROPIC_API_KEY env var)
        #[arg(long, env = "ANTHROPIC_API_KEY")]
        claude_key: Option<String>,
        /// Model name for Claude
        #[arg(long, default_value = "claude-3-5-sonnet-20241022")]
        claude_model: String,
        /// Primary comic font (default: Komika Axis like KZKT mobile)
        #[arg(short, long, default_value = "fonts/Komika Axis.ttf")]
        font: PathBuf,
        /// Secondary / CJK font for non-Latin text (default: KosugiMaru)
        #[arg(long, default_value = "fonts/KosugiMaru.ttf")]
        cjk_font: PathBuf,
    },
}

fn find_file_in_candidates(relative: &str) -> Option<PathBuf> {
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

fn ensure_model(model_path: &Path) -> Result<PathBuf> {
    if model_path.exists() {
        return Ok(model_path.to_path_buf());
    }

    if let Some(onnx) = find_file_in_candidates("models/kzkt.onnx") {
        return Ok(onnx);
    }

    if let Some(dat) = find_file_in_candidates("models/kzkt.dat") {
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.dev_help {
        print_developer_help();
        return Ok(());
    }

    let command = match cli.command {
        Some(cmd) => cmd,
        None => {
            use clap::CommandFactory;
            Cli::command().print_help()?;
            println!();
            return Ok(());
        }
    };

    match command {
        Commands::DecryptModel { source, dest } => {
            decrypt_model(&source, &dest)?;
            println!("Model successfully prepared: {:?}", dest);
        }

        Commands::Detect {
            input,
            output,
            model,
        } => {
            let model_file = ensure_model(&model)?;
            println!("[1/2] Loading model from {:?}...", model_file);
            let mut yolo = YoloModel::new(&model_file)?;

            println!("[2/2] Detecting bubbles on {:?}...", input);
            let img = image::open(&input).with_context(|| format!("Failed to open {:?}", input))?;
            let detections = yolo.detect_bubbles(&img)?;
            println!("Found {} speech bubbles.", detections.len());

            let mut rgb_img = img.to_rgb8();
            for (idx, det) in detections.iter().enumerate() {
                println!(
                    "  Bubble #{}: [{}, {}, {}, {}] (conf: {:.2})",
                    idx + 1,
                    det.x1,
                    det.y1,
                    det.x2,
                    det.y2,
                    det.conf
                );
                draw_rect(
                    &mut rgb_img,
                    det.x1,
                    det.y1,
                    det.x2,
                    det.y2,
                    Rgb([255, 0, 0]),
                    2,
                );
            }

            rgb_img.save(&output)?;
            println!("Saved detection preview to {:?}", output);
        }

        Commands::Inpaint {
            input,
            output,
            model,
        } => {
            let model_file = ensure_model(&model)?;
            println!("[1/3] Loading YOLO model...");
            let mut yolo = YoloModel::new(&model_file)?;

            println!("[2/3] Detecting bubbles on {:?}...", input);
            let img = image::open(&input)?;
            let detections = yolo.detect_bubbles(&img)?;
            println!("Found {} bubbles.", detections.len());

            println!("[3/3] Inpainting dialogue strokes with clean manga contouring...");
            let mut rgb_img = img.to_rgb8();
            inpaint_image(&mut rgb_img, &detections)?;

            rgb_img.save(&output)?;
            println!("Inpainting complete! Saved clean page to {:?}", output);
        }

        Commands::Translate {
            input,
            output,
            export,
            model,
            target_lang,
            prompt,
            batch_size,
            provider,
            gemini_key,
            openai_key,
            openai_base_url,
            openai_model,
            gemini_model,
            claude_key,
            claude_model,
            font,
            cjk_font,
        } => {
            let model_file = ensure_model(&model)?;

            println!("==> Step 1: Initializing Pipeline & Model");
            let mut yolo = YoloModel::new(&model_file)?;

            let font_bytes = if font.exists() {
                std::fs::read(&font)?
            } else if let Some(p) = find_file_in_candidates("fonts/Komika Axis.ttf") {
                std::fs::read(p)?
            } else {
                include_bytes!("../fonts/Komika Axis.ttf").to_vec()
            };

            let cjk_font_bytes = if cjk_font.exists() {
                std::fs::read(&cjk_font).ok()
            } else if let Some(p) = find_file_in_candidates("fonts/KosugiMaru.ttf") {
                std::fs::read(p).ok()
            } else {
                None
            };

            let llm_provider = match provider.to_lowercase().as_str() {
                "gemini" => {
                    let key = gemini_key
                        .context("Missing --gemini-key or GEMINI_API_KEY environment variable")?;
                    Provider::Gemini {
                        api_key: key,
                        model: gemini_model,
                    }
                }
                "openai" => {
                    let key = openai_key
                        .context("Missing --openai-key or OPENAI_API_KEY environment variable")?;
                    Provider::OpenAI {
                        api_key: key,
                        base_url: openai_base_url,
                        model: openai_model,
                    }
                }
                "ollama" => Provider::OpenAI {
                    api_key: String::new(),
                    base_url: openai_base_url,
                    model: openai_model,
                },
                "claude" => {
                    let key = claude_key.context(
                        "Missing --claude-key or ANTHROPIC_API_KEY environment variable",
                    )?;
                    Provider::Claude {
                        api_key: key,
                        model: claude_model,
                    }
                }
                other => bail!(
                    "Unknown provider: '{}'. Supported: gemini, openai, ollama, claude",
                    other
                ),
            };

            let mut final_prompt = build_translation_prompt(&target_lang);
            if let Some(ref custom) = prompt {
                final_prompt.push_str("\n\nADDITIONAL TRANSLATION RULES:\n");
                final_prompt.push_str(custom);
            }

            let ctx = TranslationContext {
                llm_provider: &llm_provider,
                prompt: &final_prompt,
                target_lang: &target_lang,
                font_bytes: &font_bytes,
                cjk_font_bytes: cjk_font_bytes.as_deref(),
                batch_size,
            };

            println!("==> Step 2: Preparing Input ({:?})", input);
            let prepared = archive::prepare_input(&input)?;

            match prepared {
                PreparedInput::SingleImage(img_path) => {
                    let out_path = output.unwrap_or_else(|| {
                        let stem = img_path.file_stem().unwrap_or_default().to_string_lossy();
                        let ext = img_path.extension().unwrap_or_default().to_string_lossy();
                        img_path
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join(format!("{}_translated.{}", stem, ext))
                    });

                    translate_page(&img_path, &out_path, &mut yolo, &ctx).await?;

                    println!("\n==> Success! Translated manga saved to {:?}", out_path);
                }

                PreparedInput::Batch {
                    images,
                    _temp_guard,
                    is_archive,
                    original_name,
                } => {
                    println!(
                        "==> Found {} pages (natural order) to translate.",
                        images.len()
                    );

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

                    let temp_output_dir = if export_as_cbz {
                        tempfile::tempdir()?.keep()
                    } else if let Some(ref out) = output {
                        out.clone()
                    } else {
                        let parent = input.parent().unwrap_or(Path::new("."));
                        parent.join(format!("{}_translated", original_name))
                    };

                    std::fs::create_dir_all(&temp_output_dir)?;

                    let mut translated_files = Vec::new();

                    for (idx, file_path) in images.iter().enumerate() {
                        let file_name = file_path.file_name().unwrap();
                        let target_path = temp_output_dir.join(file_name);
                        println!(
                            "\n[Page {}/{}] Translating: {:?}",
                            idx + 1,
                            images.len(),
                            file_name
                        );

                        if let Err(e) =
                            translate_page(file_path, &target_path, &mut yolo, &ctx).await
                        {
                            eprintln!(
                                "    [!] Error translating {:?}: {}. Copying original.",
                                file_name, e
                            );
                            let _ = std::fs::copy(file_path, &target_path);
                        } else {
                            println!("    Saved -> {:?}", target_path);
                        }
                        translated_files.push(target_path);
                    }

                    if export_as_cbz {
                        let cbz_out = if let Some(ref out) = output {
                            out.clone()
                        } else {
                            let parent = input.parent().unwrap_or(Path::new("."));
                            parent.join(format!("{}_translated.cbz", original_name))
                        };

                        println!(
                            "\n==> Packing {} pages into CBZ: {:?}",
                            translated_files.len(),
                            cbz_out
                        );
                        archive::create_cbz(&translated_files, &cbz_out)?;
                        println!("==> CBZ Export Complete: {:?}", cbz_out);
                    } else {
                        println!(
                            "\n==> All {} pages successfully saved to {:?}",
                            translated_files.len(),
                            temp_output_dir
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

struct TranslationContext<'a> {
    llm_provider: &'a Provider,
    prompt: &'a str,
    target_lang: &'a str,
    font_bytes: &'a [u8],
    cjk_font_bytes: Option<&'a [u8]>,
    batch_size: usize,
}

async fn translate_page(
    input_path: &Path,
    output_path: &Path,
    yolo: &mut YoloModel,
    ctx: &TranslationContext<'_>,
) -> Result<()> {
    let img =
        image::open(input_path).with_context(|| format!("Failed to open {:?}", input_path))?;
    let detections = yolo.detect_bubbles(&img)?;

    if detections.is_empty() {
        println!("    [Page] No dialogue bubbles detected. Copying original.");
        img.save(output_path)?;
        return Ok(());
    }

    println!("    [Page] Found {} bubbles.", detections.len());
    let orig_rgb = img.to_rgb8();
    let mut crops = Vec::new();
    for (i, det) in detections.iter().enumerate() {
        let w = det.width();
        let h = det.height();
        println!(
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

    // Chunk into batches of dialogue bubbles for LLM translation
    let mut all_translations = std::collections::HashMap::new();
    for chunk in crops.chunks(ctx.batch_size.max(1)) {
        let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
        let chunk_trans = ctx
            .llm_provider
            .translate_mosaic(&mosaic, ctx.prompt)
            .await?;
        all_translations.extend(chunk_trans);
    }

    for (k, v) in &all_translations {
        println!("      #{} -> \"{}\"", k, v);
    }

    let mut page = orig_rgb.clone();
    let inpaint_targets: Vec<kzktdk::model::yolo::Detection> = detections
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            let id = (i + 1).to_string();
            if let Some(trans) = all_translations.get(&id) {
                trans.to_uppercase() != "SKIP" && !trans.trim().is_empty()
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
    for (i, det) in detections.iter().enumerate() {
        let id = (i + 1).to_string();
        if let Some(text) = all_translations.get(&id) {
            if text.to_uppercase() == "SKIP" || text.trim().is_empty() {
                continue;
            }
            typesetter.render_bubble_text(
                &mut page,
                det,
                text,
                Some(ctx.target_lang),
                None, // Auto text & stroke colors based on bubble background
            );
        }
    }

    page.save(output_path)?;
    Ok(())
}

fn draw_rect(
    img: &mut RgbImage,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    color: Rgb<u8>,
    thickness: u32,
) {
    let (w, h) = img.dimensions();
    for t in 0..thickness {
        // Horizontal top & bottom
        for x in x1..=x2 {
            if y1 + t < h && x < w {
                img.put_pixel(x, y1 + t, color);
            }
            if y2 >= t && y2 - t < h && x < w {
                img.put_pixel(x, y2 - t, color);
            }
        }
        // Vertical left & right
        for y in y1..=y2 {
            if x1 + t < w && y < h {
                img.put_pixel(x1 + t, y, color);
            }
            if x2 >= t && x2 - t < w && y < h {
                img.put_pixel(x2 - t, y, color);
            }
        }
    }
}

fn print_developer_help() {
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
     and executes Telea Fast Marching inpainting without translation.

   • kzktdk translate <INPUT> [OPTIONS]
     Full 4-stage pipeline: Detection -> Inpainting -> Mosaic LLM Query -> Typesetting.
     Options:
       --prompt <STRING>      Append custom system prompt instructions or glossary rules
       --batch-size <N>       Max dialogue bubbles grouped per LLM request (default: 15)
       -t, --target-lang      Target language (default: English)
       -p, --provider         LLM provider (gemini, openai, ollama, claude)

2. PIPELINE INTERNALS & THRESHOLDS:

   • YOLOv8 Detection:
     - Input size: 640x640 letterbox normalized NCHW
     - Stage 1: conf >= 0.28, iou = 0.45
     - Stage 2: conf >= 0.18, iou = 0.55
     - Stage 3: conf >= 0.10, iou = 0.65
     - False-giant filter: boxes >= 92% of page area are discarded
     - Aspect ratio guard: width:height ratio > 12:1 is rejected

   • Telea Inpainting:
     - Delta luminance threshold: delta L >= 45 from median interior background
     - Structuring element: circular r = 2 for edge fringe dilation
     - Pure Rust Eikonal FMM solver from `inpaint` crate

   • Mosaic Builder:
     - Bubble crops composite vertically with red ID labels (#1, #2, ...)
     - Batching: chunks of up to 15 bubbles (configurable via --batch-size)

   • Diamond Elliptical Typesetter:
     - Curvature scale: max(0.82, sqrt(max(0.20, 1.0 - 0.40 * y^2)))
     - Dynamic line wrapping based on curved manga bubble geometry
     - Phonotactic syllable hyphenation for Indonesian (EYD) and English (affixes)

3. MODEL & ASSET SEARCH RESOLUTION:
   Candidate paths checked in order:
   1. Explicit CLI flag (--model, --font)
   2. Current working directory (./models/kzkt.onnx)
   3. Executable directory ($EXE_DIR/models/kzkt.onnx)
   4. Parent directory ($EXE_DIR/../models/kzkt.onnx)
   5. User data directory ($XDG_DATA_HOME/kzktdk, %LOCALAPPDATA%\kzktdk, or $HOME/.local/share/kzktdk)
   6. Auto-decryption from kzkt.dat if onnx is missing
   7. Embedded fonts (Komika Axis TTF embedded in binary as fallback)
================================================================================
"#
    );
}
