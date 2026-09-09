use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use image::{GenericImageView, Rgb, RgbImage};
use std::path::{Path, PathBuf};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

use kzktdk::archive::{self, PreparedInput};
use kzktdk::cache::{TranslationCache, prompt_signature};
use kzktdk::inpaint::inpaint_image;
use kzktdk::metadata::{self, PageEditData};
use kzktdk::model::decrypt::decrypt_model;
use kzktdk::model::yolo::YoloModel;
use kzktdk::translation::{CropItem, MosaicBuilder, Provider, ProviderChain, RateLimiter, build_translation_prompt, translate_with_chain};
use kzktdk::typesetting::Typesetter;

const MAIN_HELP_TEMPLATE: &str = "\
{name} <command> [options]

Usage:

  kzktdk translate <archive.cbz>                 Translate comic archive (.cbz / .zip / .epub)
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

    /// Detect speech bubbles and draw bounding boxes (supports batch folder/cbz)
    Detect {
        /// Input path: image, folder, or archive (cbz/zip/epub)
        input: PathBuf,
        /// Output path: file for single, folder for batch
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// ONNX model path (will auto-decrypt models/kzkt.dat if missing)
        #[arg(short, long, default_value = "models/kzkt.onnx")]
        model: PathBuf,
        /// Export detections to JSON file
        #[arg(long)]
        json: Option<PathBuf>,
        /// Jobs for batch parallel: auto or number
        #[arg(long, default_value = "auto")]
        jobs: String,
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
  kzktdk translate \"chapter_01.cbz\" --provider ollama --openai-base-url \"http://localhost:11434/v1\" --openai-model \"llama3.2-vision\"
  kzktdk translate \"chapter.cbz\" --fallback-provider openai --rate-limit 3 --jobs auto")]
    Translate {
        /// Input path: image (.jpg/.png/.webp), folder, or comic archive (.cbz/.zip/.epub)
        input: Option<PathBuf>,
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
        /// Fallback providers comma-separated (e.g. openai,claude)
        #[arg(long)]
        fallback_provider: Option<String>,
        /// Rate limit RPS
        #[arg(long, default_value = "3")]
        rate_limit: u32,
        /// Jobs for batch parallel: auto or number (default auto = num_cpus)
        #[arg(long, default_value = "auto")]
        jobs: String,
        /// Disable translation cache
        #[arg(long)]
        no_cache: bool,
        /// Clear translation cache and exit
        #[arg(long)]
        clear_cache: bool,
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
        /// Save metadata sidecar .kedit.json per page + project.kedit.json
        #[arg(long)]
        save_metadata: bool,
        /// Custom metadata directory (default: same as output)
        #[arg(long)]
        metadata_dir: Option<PathBuf>,
    },

    /// Metadata operations for editor backend
    Metadata {
        #[command(subcommand)]
        cmd: MetadataCmd,
    },

    /// Font registry operations
    Font {
        #[command(subcommand)]
        cmd: FontCmd,
    },
}

#[derive(Subcommand)]
enum MetadataCmd {
    /// Export detections to JSON without LLM (cheap)
    Export {
        /// Input path: image, folder, or archive
        input: PathBuf,
        /// Output JSON file
        #[arg(short, long)]
        json: PathBuf,
        /// ONNX model path
        #[arg(short, long, default_value = "models/kzkt.onnx")]
        model: PathBuf,
    },
    /// Render image from metadata JSON without LLM
    Render {
        /// Original image path
        image: PathBuf,
        /// Metadata JSON file (.kedit.json)
        #[arg(long)]
        metadata: PathBuf,
        /// Output rendered image
        #[arg(short, long)]
        output: PathBuf,
        /// Font path
        #[arg(short, long, default_value = "fonts/Komika Axis.ttf")]
        font: PathBuf,
        /// CJK font path
        #[arg(long, default_value = "fonts/KosugiMaru.ttf")]
        cjk_font: PathBuf,
    },
    /// Preview single bubble text without saving JSON (for live editor)
    Preview {
        /// Original image path
        image: PathBuf,
        /// Metadata JSON file
        #[arg(long)]
        metadata: PathBuf,
        /// Bubble ID
        #[arg(long)]
        id: String,
        /// New text to preview
        #[arg(long)]
        text: String,
        /// Output preview image
        #[arg(short, long)]
        output: PathBuf,
        /// Font path
        #[arg(short, long, default_value = "fonts/Komika Axis.ttf")]
        font: PathBuf,
        /// CJK font path
        #[arg(long, default_value = "fonts/KosugiMaru.ttf")]
        cjk_font: PathBuf,
    },
    /// Edit metadata JSON (set translated text or bbox)
    Edit {
        /// Metadata JSON file
        json: PathBuf,
        /// Set translated text: "ID=New Text" (can repeat)
        #[arg(long)]
        set: Vec<String>,
        /// Set bbox: "ID=x1,y1,x2,y2" (can repeat)
        #[arg(long)]
        bbox: Vec<String>,
        /// Add new bubble: "ID=x1,y1,x2,y2" or "ID=x1,y1,x2,y2=text" (can repeat)
        #[arg(long)]
        add: Vec<String>,
        /// Delete bubble by ID (can repeat)
        #[arg(long)]
        delete: Vec<String>,
        /// Set font family per bubble: "ID=FontName" (can repeat, empty to clear)
        #[arg(long, value_name = "ID=FontName")]
        font_family: Vec<String>,
        /// Set font size per bubble: "ID=14.5" (can repeat, empty to clear auto)
        #[arg(long, value_name = "ID=Size")]
        font_size: Vec<String>,
        /// Set text color per bubble: "ID=R,G,B" (can repeat, empty to clear)
        #[arg(long, value_name = "ID=R,G,B")]
        text_color: Vec<String>,
        /// Set stroke color per bubble: "ID=R,G,B" (can repeat, empty to clear)
        #[arg(long, value_name = "ID=R,G,B")]
        stroke_color: Vec<String>,
        /// Set align per bubble: "ID=left|center|right" (can repeat, empty to clear)
        #[arg(long, value_name = "ID=Align")]
        align: Vec<String>,
        /// Set edited flag: "ID=true/false" (can repeat)
        #[arg(long, value_name = "ID=Bool")]
        edited: Vec<String>,
    },
    /// Validate metadata JSON
    Validate {
        /// JSON file to validate
        json: PathBuf,
    },
}

#[derive(Subcommand)]
enum FontCmd {
    /// Import font file into registry
    Import {
        /// Path to TTF/OTF file
        path: PathBuf,
        /// Custom name (default: file stem)
        #[arg(long)]
        name: Option<String>,
    },
    /// List imported fonts
    List,
    /// Remove font from registry
    Remove {
        /// Font name
        name: String,
    },
    /// Set default font (global)
    SetDefault {
        /// Font name (from registry) or path
        name: String,
        /// Set for CJK instead of Latin
        #[arg(long)]
        cjk: bool,
    },
    /// Get default font
    GetDefault,
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

fn build_provider(
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

fn parse_jobs(jobs_str: &str) -> usize {
    if jobs_str.eq_ignore_ascii_case("auto") {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    } else if let Ok(n) = jobs_str.parse::<usize>() {
        n.max(1)
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    }
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
            json,
            jobs,
        } => {
            let model_file = ensure_model(&model)?;
            println!("[1/2] Loading model from {:?}...", model_file);
            let mut yolo = YoloModel::new(&model_file)?;
            let jobs_num = parse_jobs(&jobs);

            let prepared = archive::prepare_input(&input)?;
            match prepared {
                PreparedInput::SingleImage(img_path) => {
                    let out_path = output.clone().unwrap_or_else(|| PathBuf::from("detected.png"));
                    // If output is a directory, place file inside
                    let out_path = if out_path.is_dir() || out_path.to_string_lossy().ends_with('/') {
                        std::fs::create_dir_all(&out_path)?;
                        out_path.join(img_path.file_name().unwrap())
                    } else {
                        if let Some(parent) = out_path.parent() { std::fs::create_dir_all(parent)?; }
                        out_path
                    };
                    println!("[2/2] Detecting bubbles on {:?}...", img_path);
                    let img = image::open(&img_path).with_context(|| format!("Failed to open {:?}", img_path))?;
                    let (w, h) = (img.width(), img.height());
                    let detections = yolo.detect_bubbles(&img)?;
                    println!("Found {} speech bubbles.", detections.len());
                    let mut rgb_img = img.to_rgb8();
                    for (idx, det) in detections.iter().enumerate() {
                        println!("  Bubble #{}: [{}, {}, {}, {}] (conf: {:.2})", idx+1, det.x1, det.y1, det.x2, det.y2, det.conf);
                        draw_rect(&mut rgb_img, det.x1, det.y1, det.x2, det.y2, Rgb([255,0,0]), 2);
                    }
                    rgb_img.save(&out_path)?;
                    println!("Saved detection preview to {:?}", out_path);
                    if let Some(json_path) = json {
                        let data = metadata::PageEditData::new(
                            img_path.file_name().unwrap().to_string_lossy().to_string(),
                            w, h, "English".to_string(), "classic".to_string(),
                            detections.iter().enumerate().map(|(i,d)| metadata::Bubble {
                                id: (i+1).to_string(), bbox: [d.x1,d.y1,d.x2,d.y2], conf: d.conf, translated: String::new(), bg_color: None, style: None, edited: false
                            }).collect()
                        );
                        metadata::save_page_metadata(&json_path, &data)?;
                        println!("Saved JSON to {:?}", json_path);
                    }
                }
                PreparedInput::Batch { images, _temp_guard: _, is_archive: _, original_name } => {
                    let out_dir = output.unwrap_or_else(|| PathBuf::from("detected"));
                    std::fs::create_dir_all(&out_dir)?;
                    println!("[2/2] Detecting bubbles on {} pages (jobs={})...", images.len(), jobs_num);
                    let mut all_pages = Vec::new();
                    if jobs_num <= 1 {
                        for (idx, p) in images.iter().enumerate() {
                            let img = image::open(p).with_context(|| format!("Failed to open {:?}", p))?;
                            let (w,h) = (img.width(), img.height());
                            let dets = yolo.detect_bubbles(&img)?;
                            println!("[Page {}/{}] {:?}: {} bubbles", idx+1, images.len(), p.file_name().unwrap(), dets.len());
                            let mut rgb = img.to_rgb8();
                            for d in &dets { draw_rect(&mut rgb, d.x1, d.y1, d.x2, d.y2, Rgb([255,0,0]), 2); }
                            let out = out_dir.join(p.file_name().unwrap());
                            rgb.save(&out)?;
                            if json.is_some() {
                                all_pages.push(metadata::PageEditData::new(
                                    p.file_name().unwrap().to_string_lossy().to_string(), w, h, "English".to_string(), "classic".to_string(),
                                    dets.iter().enumerate().map(|(i,d)| metadata::Bubble { id: (i+1).to_string(), bbox: [d.x1,d.y1,d.x2,d.y2], conf: d.conf, translated: String::new(), bg_color: None, style: None, edited: false }).collect()
                                ));
                            }
                        }
                    } else {
                        // For batch detect, sequential yolo is fine; parallel would need Send. Keep sequential for now with progress.
                        for (idx, p) in images.iter().enumerate() {
                            let img = image::open(p).with_context(|| format!("Failed to open {:?}", p))?;
                            let (w,h) = (img.width(), img.height());
                            let dets = yolo.detect_bubbles(&img)?;
                            println!("[Page {}/{}] {:?}: {} bubbles", idx+1, images.len(), p.file_name().unwrap(), dets.len());
                            let mut rgb = img.to_rgb8();
                            for d in &dets { draw_rect(&mut rgb, d.x1, d.y1, d.x2, d.y2, Rgb([255,0,0]), 2); }
                            let out = out_dir.join(p.file_name().unwrap());
                            rgb.save(&out)?;
                            if json.is_some() {
                                all_pages.push(metadata::PageEditData::new(
                                    p.file_name().unwrap().to_string_lossy().to_string(), w, h, "English".to_string(), "classic".to_string(),
                                    dets.iter().enumerate().map(|(i,d)| metadata::Bubble { id: (i+1).to_string(), bbox: [d.x1,d.y1,d.x2,d.y2], conf: d.conf, translated: String::new(), bg_color: None, style: None, edited: false }).collect()
                                ));
                            }
                        }
                    }
                    println!("Saved {} previews to {:?}", images.len(), out_dir);
                    if let Some(json_path) = json {
                        let v = serde_json::to_value(&all_pages).unwrap();
                        std::fs::create_dir_all(json_path.parent().unwrap_or(Path::new(".")))?;
                        std::fs::write(&json_path, serde_json::to_string_pretty(&v).unwrap())?;
                        println!("Saved batch JSON to {:?}", json_path);
                    }
                }
            }
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

            println!("[3/3] Inpainting dialogue strokes with clean manga contouring (parallel)...");
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
            fallback_provider,
            rate_limit,
            jobs,
            no_cache,
            clear_cache,
            gemini_key,
            openai_key,
            openai_base_url,
            openai_model,
            gemini_model,
            claude_key,
            claude_model,
            font,
            cjk_font,
            save_metadata,
            metadata_dir,
        } => {
            if clear_cache {
                let cache = TranslationCache::open()?;
                cache.clear()?;
                println!("[Cache] Cleared translation cache");
                return Ok(());
            }
            let input = input.context("Missing <INPUT> path (required unless --clear-cache)")?;

            let model_file = ensure_model(&model)?;

            println!("==> Step 1: Initializing Pipeline & Model");
            // Resolve font via registry/config (global + per-bubble support)
            let font_bytes = {
                let font_str = font.to_string_lossy().to_string();
                // If font is a registry name (not a file), try resolve
                if !font.exists() && !font_str.contains('/') && !font_str.contains('\\') {
                    if kzktdk::font::FontRegistry::resolve(&font_str).is_ok() {
                        // Read from registry path
                        let list = kzktdk::font::FontRegistry::list();
                        if let Some(info) = list.iter().find(|f| f.name == font_str) {
                            if let Ok(b) = std::fs::read(&info.path) { b } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() }
                        } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() }
                    } else if let Some(def) = kzktdk::font::AppConfig::get_latin() {
                        if kzktdk::font::FontRegistry::resolve(&def).is_ok() {
                            let list = kzktdk::font::FontRegistry::list();
                            if let Some(info) = list.iter().find(|f| f.name == def) {
                                if let Ok(b) = std::fs::read(&info.path) { b } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() }
                            } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() }
                        } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() }
                    } else if let Some(p) = find_file_in_candidates("fonts/Komika Axis.ttf") {
                        std::fs::read(p)?
                    } else {
                        include_bytes!("../fonts/Komika Axis.ttf").to_vec()
                    }
                } else if font.exists() {
                    std::fs::read(&font)?
                } else if let Some(p) = find_file_in_candidates("fonts/Komika Axis.ttf") {
                    std::fs::read(p)?
                } else {
                    include_bytes!("../fonts/Komika Axis.ttf").to_vec()
                }
            };

            let cjk_font_bytes = if cjk_font.exists() {
                std::fs::read(&cjk_font).ok()
            } else {
                let cjk_str = cjk_font.to_string_lossy().to_string();
                if !cjk_str.is_empty() && !cjk_str.contains('/') && !cjk_str.contains('\\') && kzktdk::font::FontRegistry::resolve(&cjk_str).is_ok() {
                    let list = kzktdk::font::FontRegistry::list();
                    if let Some(info) = list.iter().find(|f| f.name == cjk_str) {
                        std::fs::read(&info.path).ok()
                    } else { None }
                } else if let Some(def) = kzktdk::font::AppConfig::get_cjk() {
                    let list = kzktdk::font::FontRegistry::list();
                    if let Some(info) = list.iter().find(|f| f.name == def) {
                        std::fs::read(&info.path).ok()
                    } else if let Some(p) = find_file_in_candidates("fonts/KosugiMaru.ttf") {
                        std::fs::read(p).ok()
                    } else { None }
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
                for fb_name in fb_str.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
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
            let provider_chain = ProviderChain {
                primary,
                fallbacks,
            };
            let rate_limiter = RateLimiter::new(rate_limit);
            let jobs_num = parse_jobs(&jobs);
            println!("  Jobs: {} (batch parallel), RateLimit: {} RPS", jobs_num, rate_limit);
            if !provider_chain.fallbacks.is_empty() {
                let fb_names: Vec<_> = provider_chain.fallbacks.iter().map(|p| p.name()).collect();
                println!("  Fallbacks: {}", fb_names.join(", "));
            }

            let use_cache = !no_cache;
            let prompt_sig = prompt_signature(prompt.as_deref());
            let cache_opt = if use_cache {
                match TranslationCache::open() {
                    Ok(c) => {
                        println!("  Cache: enabled (prompt_sig={})", prompt_sig);
                        Some(Arc::new(c))
                    }
                    Err(e) => {
                        eprintln!("  [Warn] Cache disabled: {}", e);
                        None
                    }
                }
            } else {
                println!("  Cache: disabled");
                None
            };

            let mut final_prompt = build_translation_prompt(&target_lang);
            if let Some(ref custom) = prompt {
                final_prompt.push_str("\n\nADDITIONAL TRANSLATION RULES:\n");
                final_prompt.push_str(custom);
            }

            // We need to defer Yolo creation for parallel batch: use Arc<Mutex> or per-task.
            // For simplicity, create one for single image path, and for batch we create per task via model_file clone.
            println!("==> Step 2: Preparing Input ({:?})", input);
            let prepared = archive::prepare_input(&input)?;

            match prepared {
                PreparedInput::SingleImage(img_path) => {
                    let mut yolo = YoloModel::new(&model_file)?;
                    let out_path = output.unwrap_or_else(|| {
                        let stem = img_path.file_stem().unwrap_or_default().to_string_lossy();
                        let ext = img_path.extension().unwrap_or_default().to_string_lossy();
                        img_path
                            .parent()
                            .unwrap_or(Path::new("."))
                            .join(format!("{}_translated.{}", stem, ext))
                    });

                    let ctx = TranslationContext {
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
                    };
                    translate_page(&img_path, &out_path, &mut yolo, &ctx).await?;

                    // Save project.kedit.json for single image if requested
                    if save_metadata {
                        let meta_dir = metadata_dir.clone().unwrap_or_else(|| out_path.parent().unwrap_or(Path::new(".")).to_path_buf());
                        let proj = PathBuf::from(&meta_dir).join("project.kedit.json");
                        let sidecar = meta_dir.join(format!("{}.kedit.json", out_path.file_stem().unwrap_or_default().to_string_lossy()));
                        metadata::save_project(&proj, &[sidecar], Some(target_lang.clone()))?;
                        println!("[Metadata] Project saved to {:?}", proj);
                    }

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

                    // Parallel batch translation
                    if jobs_num <= 1 || images.len() <= 1 {
                        // Sequential fallback
                        let mut translated_files = Vec::new();
                        let mut yolo = YoloModel::new(&model_file)?;
                        let ctx = TranslationContext {
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
                        };
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
                        if save_metadata {
                            let meta_dir = metadata_dir.clone().unwrap_or_else(|| temp_output_dir.clone());
                            std::fs::create_dir_all(&meta_dir)?;
                            let sidecars: Vec<PathBuf> = translated_files.iter().map(|p| meta_dir.join(format!("{}.kedit.json", p.file_stem().unwrap_or_default().to_string_lossy()))).collect();
                            let proj = meta_dir.join("project.kedit.json");
                            let _ = metadata::save_project(&proj, &sidecars, Some(target_lang.clone()));
                            println!("[Metadata] Project saved to {:?}", proj);
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
                    } else {
                        // Parallel with JoinSet + Semaphore
                        use tokio::sync::Semaphore;
                        let semaphore = Arc::new(Semaphore::new(jobs_num));
                        let mut join_set = tokio::task::JoinSet::new();
                        let completed = Arc::new(AtomicUsize::new(0));
                        let total = images.len();
                        let model_file_arc = Arc::new(model_file.clone());
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

                        for (idx, file_path) in images.iter().enumerate() {
                            let sem = semaphore.clone();
                            let completed = completed.clone();
                            let out_dir = temp_output_dir.clone();
                            let file_path = file_path.clone();
                            let model_file = model_file_arc.clone();
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
                            join_set.spawn(async move {
                                let _permit = sem.acquire_owned().await.unwrap();
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
                                ).expect("primary provider build failed");
                                let mut fallbacks = Vec::new();
                                if let Some(fb) = fallback_str_c {
                                    for fb_name in fb.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                                        if fb_name.eq_ignore_ascii_case(&provider_name) { continue; }
                                        if let Ok(p) = build_provider(fb_name, &gemini_key_c, &gemini_model_c, &openai_key_c, &openai_base_c, &openai_model_c, &claude_key_c, &claude_model_c) {
                                            fallbacks.push(p);
                                        }
                                    }
                                }
                                let chain = ProviderChain { primary, fallbacks };
                                let limiter = RateLimiter::new(rate_limit_c);
                                let mut yolo = YoloModel::new(model_file.as_path()).expect("yolo load failed");
                                let file_name = file_path.file_name().unwrap().to_owned();
                                let target_path = out_dir.join(&file_name);
                                // per-task cache (open fresh connection to avoid !Send)
                                let cache_local: Option<TranslationCache> = if use_cache_flag {
                                    TranslationCache::open().ok()
                                } else {
                                    None
                                };
                                let ctx = TranslationContext {
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
                                };
                                let res = translate_page(&file_path, &target_path, &mut yolo, &ctx).await;
                                let done = completed.fetch_add(1, Ordering::SeqCst) + 1;
                                match res {
                                    Ok(_) => println!("[Page {}/{}] Done {:?} -> {:?}", done, total, file_name, target_path),
                                    Err(e) => {
                                        eprintln!("[Page {}/{}] [!] Error {:?}: {}. Copying original.", done, total, file_name, e);
                                        let _ = std::fs::copy(&file_path, &target_path);
                                    }
                                }
                                (target_path, idx)
                            });
                        }

                        let mut results: Vec<(PathBuf, usize)> = Vec::new();
                        while let Some(r) = join_set.join_next().await {
                            if let Ok(v) = r { results.push(v); }
                        }
                        results.sort_by_key(|(_, idx)| *idx);
                        let translated_files: Vec<PathBuf> = results.into_iter().map(|(p, _)| p).collect();

                        if save_meta_flag {
                            let meta_dir = meta_dir_opt.clone().unwrap_or_else(|| temp_output_dir.clone());
                            std::fs::create_dir_all(&meta_dir)?;
                            let sidecars: Vec<PathBuf> = translated_files.iter().map(|p| meta_dir.join(format!("{}.kedit.json", p.file_stem().unwrap_or_default().to_string_lossy()))).collect();
                            let proj = meta_dir.join("project.kedit.json");
                            let _ = metadata::save_project(&proj, &sidecars, Some(target_lang.clone()));
                            println!("[Metadata] Project saved to {:?}", proj);
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

        Commands::Metadata { cmd } => match cmd {
            MetadataCmd::Export { input, json, model } => {
                let model_file = ensure_model(&model)?;
                let mut yolo = YoloModel::new(&model_file)?;
                let prepared = archive::prepare_input(&input)?;
                let mut pages = Vec::new();
                let images = match prepared {
                    PreparedInput::SingleImage(p) => vec![p],
                    PreparedInput::Batch { images, .. } => images,
                };
                for p in &images {
                    let img = image::open(p).with_context(|| format!("Failed to open {:?}", p))?;
                    let (w, h) = (img.width(), img.height());
                    let dets = yolo.detect_bubbles(&img)?;
                    println!("{:?}: {} bubbles", p.file_name().unwrap(), dets.len());
                    pages.push(PageEditData::new(
                        p.file_name().unwrap().to_string_lossy().to_string(),
                        w, h, "English".to_string(), "classic".to_string(),
                        dets.iter().enumerate().map(|(i,d)| metadata::Bubble { id: (i+1).to_string(), bbox: [d.x1,d.y1,d.x2,d.y2], conf: d.conf, translated: String::new(), bg_color: None, style: None, edited: false }).collect()
                    ));
                }
                let v = serde_json::to_value(&pages).unwrap();
                std::fs::create_dir_all(json.parent().unwrap_or(Path::new(".")))?;
                std::fs::write(&json, serde_json::to_string_pretty(&v).unwrap())?;
                println!("Exported {} pages to {:?}", pages.len(), json);
            }
            MetadataCmd::Render { image, metadata, output, font, cjk_font } => {
                let data = metadata::load_page_metadata(&metadata)?;
                let img = image::open(&image).with_context(|| format!("Failed to open {:?}", image))?;
                let mut rgb = img.to_rgb8();
                // inpaint targets where translated != SKIP
                let dets: Vec<kzktdk::model::yolo::Detection> = data.bubbles.iter().filter(|b| b.translated.to_uppercase() != "SKIP" && !b.translated.trim().is_empty()).map(|b| kzktdk::model::yolo::Detection { x1: b.bbox[0], y1: b.bbox[1], x2: b.bbox[2], y2: b.bbox[3], conf: b.conf }).collect();
                if !dets.is_empty() {
                    inpaint_image(&mut rgb, &dets)?;
                }
                let font_bytes = if font.exists() { std::fs::read(&font)? } else {
                    // Try registry name
                    let fstr = font.to_string_lossy().to_string();
                    if !fstr.is_empty() && kzktdk::font::FontRegistry::resolve(&fstr).is_ok() {
                        let list = kzktdk::font::FontRegistry::list();
                        if let Some(info) = list.iter().find(|f| f.name == fstr) {
                            std::fs::read(&info.path).unwrap_or_else(|_| include_bytes!("../fonts/Komika Axis.ttf").to_vec())
                        } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() }
                    } else if let Some(p) = find_file_in_candidates("fonts/Komika Axis.ttf") { std::fs::read(p)? } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() }
                };
                let cjk_bytes = if cjk_font.exists() { std::fs::read(&cjk_font).ok() } else if let Some(p) = find_file_in_candidates("fonts/KosugiMaru.ttf") { std::fs::read(p).ok() } else { None };
                let global_typesetter = Typesetter::new(&font_bytes, cjk_bytes.as_deref())?;
                for b in &data.bubbles {
                    if b.translated.to_uppercase() == "SKIP" || b.translated.trim().is_empty() { continue; }
                    let det = kzktdk::model::yolo::Detection { x1: b.bbox[0], y1: b.bbox[1], x2: b.bbox[2], y2: b.bbox[3], conf: b.conf };
                    // Per-bubble font override + style (font_size, color, align, etc.)
                    if let Some(style) = &b.style {
                        if let Some(ref fname) = style.font_family {
                            let mut custom_bytes: Option<Vec<u8>> = None;
                            if kzktdk::font::FontRegistry::resolve(fname).is_ok() {
                                let list = kzktdk::font::FontRegistry::list();
                                if let Some(info) = list.iter().find(|f| &f.name == fname) {
                                    custom_bytes = std::fs::read(&info.path).ok();
                                }
                            } else if Path::new(fname).exists() {
                                custom_bytes = std::fs::read(fname).ok();
                            }
                            if let Some(bytes) = custom_bytes {
                                if let Ok(ts) = Typesetter::new(&bytes, cjk_bytes.as_deref()) {
                                    ts.render_bubble_text_with_style(&mut rgb, &det, &b.translated, Some(&data.target_lang), None, Some(style));
                                    continue;
                                }
                            }
                        }
                    }
                    global_typesetter.render_bubble_text_with_style(&mut rgb, &det, &b.translated, Some(&data.target_lang), None, b.style.as_ref());
                }
                rgb.save(&output)?;
                println!("Rendered {:?} -> {:?}", image, output);
            }
            MetadataCmd::Edit { json, set, bbox, add, delete, font_family, font_size, text_color, stroke_color, align, edited } => {
                let mut data = metadata::load_page_metadata(&json)?;
                for s in set {
                    if let Some((id, txt)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            b.translated = txt.to_string();
                            b.edited = true;
                            println!("Set {} = \"{}\"", id, txt);
                        } else {
                            eprintln!("Bubble {} not found", id);
                        }
                    }
                }
                for s in bbox {
                    if let Some((id, coords)) = s.split_once('=') {
                        let parts: Vec<u32> = coords.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                        if parts.len() == 4 {
                            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                                b.bbox = [parts[0], parts[1], parts[2], parts[3]];
                                println!("Set bbox {} = {:?}", id, b.bbox);
                            } else {
                                eprintln!("Bubble {} not found", id);
                            }
                        } else {
                            eprintln!("Invalid bbox format for {}: expected x1,y1,x2,y2", id);
                        }
                    }
                }
                for s in add {
                    // Format: ID=x1,y1,x2,y2 or ID=x1,y1,x2,y2=text (text may contain '=')
                    if let Some((id, rest)) = s.split_once('=') {
                        // Try to split rest into bbox and optional text at first '=' after bbox
                        // bbox has 3 commas, so find position of text separator: look for pattern with 3 commas before '='
                        let (bbox_str, txt) = if let Some(eq_pos) = rest.find('=') {
                            let candidate_bbox = &rest[..eq_pos];
                            let comma_cnt = candidate_bbox.matches(',').count();
                            if comma_cnt == 3 {
                                (&rest[..eq_pos], &rest[eq_pos+1..])
                            } else {
                                (rest, "")
                            }
                        } else {
                            (rest, "")
                        };
                        if data.bubbles.iter().any(|b| b.id == id) {
                            eprintln!("Bubble {} already exists, skip add", id);
                            continue;
                        }
                        let parts: Vec<u32> = bbox_str.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                        if parts.len() != 4 {
                            eprintln!("Invalid bbox for add {}: expected x1,y1,x2,y2", id);
                            continue;
                        }
                        if parts[0] >= parts[2] || parts[1] >= parts[3] {
                            eprintln!("Invalid bbox for add {}: x1<x2 and y1<y2 required", id);
                            continue;
                        }
                        data.bubbles.push(metadata::Bubble { id: id.to_string(), bbox: [parts[0],parts[1],parts[2],parts[3]], conf: 1.0, translated: txt.to_string(), bg_color: None, style: None, edited: false });
                        println!("Added bubble {} bbox {:?} text \"{}\"", id, [parts[0],parts[1],parts[2],parts[3]], txt);
                    } else {
                        eprintln!("Invalid --add format: expected ID=x1,y1,x2,y2[=text] got '{}'", s);
                    }
                }
                for id in delete {
                    let before = data.bubbles.len();
                    data.bubbles.retain(|b| b.id != id);
                    if data.bubbles.len() < before {
                        println!("Deleted bubble {}", id);
                    } else {
                        eprintln!("Bubble {} not found for delete", id);
                    }
                }
                for s in font_family {
                    if let Some((id, font)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if font.is_empty() {
                                if let Some(style) = b.style.as_mut() { style.font_family = None; }
                                println!("Cleared font for {}", id);
                            } else {
                                let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                                style.font_family = Some(font.to_string());
                                println!("Set font {} = {}", id, font);
                            }
                        } else {
                            eprintln!("Bubble {} not found for font_family", id);
                        }
                    }
                }
                for s in font_size {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() {
                                if let Some(style) = b.style.as_mut() { style.font_size = None; }
                                println!("Cleared font_size for {}", id);
                            } else if let Ok(f) = val.parse::<f32>() {
                                let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                                style.font_size = Some(f);
                                println!("Set font_size {} = {}", id, f);
                            } else { eprintln!("Invalid font_size for {}: {}", id, val); }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in text_color {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() {
                                if let Some(style) = b.style.as_mut() { style.text_color = None; }
                                println!("Cleared text_color for {}", id);
                            } else {
                                let parts: Vec<u8> = val.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                                if parts.len()==3 {
                                    let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                                    style.text_color = Some([parts[0],parts[1],parts[2]]);
                                    println!("Set text_color {} = {:?}", id, style.text_color);
                                } else { eprintln!("Invalid text_color for {}: expected R,G,B", id); }
                            }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in stroke_color {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() {
                                if let Some(style) = b.style.as_mut() { style.stroke_color = None; }
                                println!("Cleared stroke_color for {}", id);
                            } else {
                                let parts: Vec<u8> = val.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                                if parts.len()==3 {
                                    let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                                    style.stroke_color = Some([parts[0],parts[1],parts[2]]);
                                    println!("Set stroke_color {} = {:?}", id, style.stroke_color);
                                } else { eprintln!("Invalid stroke_color for {}: expected R,G,B", id); }
                            }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in align {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() {
                                if let Some(style) = b.style.as_mut() { style.align = None; }
                                println!("Cleared align for {}", id);
                            } else {
                                let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                                style.align = Some(val.to_string());
                                println!("Set align {} = {}", id, val);
                            }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in edited {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if let Ok(v) = val.parse::<bool>() { b.edited = v; println!("Set edited {} = {}", id, v); }
                            else { eprintln!("Invalid edited for {}: expected true/false", id); }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                metadata::save_page_metadata(&json, &data)?;
                println!("Saved edited metadata to {:?}", json);
            }
            MetadataCmd::Validate { json } => {
                let data = metadata::load_page_metadata(&json)?;
                println!("Valid PageEditData v{}: {} bubbles, page={} ({}x{})", data.version, data.bubbles.len(), data.page, data.width, data.height);
                // Also try project
                if let Ok(proj) = metadata::load_project(&json) {
                    println!("Valid Project v{}: {} pages", proj.version, proj.pages.len());
                }
            }
            MetadataCmd::Preview { image, metadata, id, text, output, font, cjk_font } => {
                let mut data = metadata::load_page_metadata(&metadata)?;
                let bubble = data.bubbles.iter().find(|b| b.id == id).cloned().context(format!("Bubble {} not found", id))?;
                let mut rgb = image::open(&image).with_context(|| format!("Failed to open {:?}", image))?.to_rgb8();
                // Inpaint just this bubble
                let det = kzktdk::model::yolo::Detection { x1: bubble.bbox[0], y1: bubble.bbox[1], x2: bubble.bbox[2], y2: bubble.bbox[3], conf: bubble.conf };
                inpaint_image(&mut rgb, &[det.clone()])?;
                // Load fonts
                let font_bytes = if font.exists() { std::fs::read(&font)? } else if let Some(p) = find_file_in_candidates("fonts/Komika Axis.ttf") { std::fs::read(p)? } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() };
                let cjk_bytes = if cjk_font.exists() { std::fs::read(&cjk_font).ok() } else if let Some(p) = find_file_in_candidates("fonts/KosugiMaru.ttf") { std::fs::read(p).ok() } else { None };
                // Check bubble style for font override in preview
                let style = bubble.style.clone();
                let custom_bytes: Option<Vec<u8>> = if let Some(s) = &style {
                    if let Some(fname) = &s.font_family {
                        kzktdk::font::FontRegistry::list().iter().find(|f| &f.name == fname).and_then(|info| std::fs::read(&info.path).ok())
                    } else { None }
                } else { None };
                let typesetter = if let Some(ref bytes) = custom_bytes {
                    Typesetter::new(bytes, cjk_bytes.as_deref()).unwrap_or_else(|_| Typesetter::new(&font_bytes, cjk_bytes.as_deref()).unwrap())
                } else {
                    Typesetter::new(&font_bytes, cjk_bytes.as_deref())?
                };
                // Create a temporary BubbleStyle for preview text with same style but new text
                let preview_style = style.clone();
                typesetter.render_bubble_text_with_style(&mut rgb, &det, &text, Some(&data.target_lang), None, preview_style.as_ref());
                rgb.save(&output)?;
                println!("Preview bubble {} -> {:?}", id, output);
                // Also show fit info
                let info_style = preview_style.as_ref();
                println!("Preview text: \"{}\" (font_size override {:?}, align {:?})", text, info_style.and_then(|s| s.font_size), info_style.and_then(|s| s.align.as_ref()));
            }
        }

        Commands::Font { cmd } => match cmd {
            FontCmd::Import { path, name } => {
                let imported = kzktdk::font::FontRegistry::import(&path, name)?;
                println!("Imported font '{}' from {:?}", imported, path);
            }
            FontCmd::List => {
                let fonts = kzktdk::font::FontRegistry::list();
                if fonts.is_empty() {
                    println!("No imported fonts. Default: Komika Axis, KosugiMaru (embedded)");
                } else {
                    println!("Imported fonts:");
                    for f in fonts { println!("  - {} ({} has_cjk={})", f.name, f.path, f.has_cjk); }
                }
                println!("Default: Komika Axis, KosugiMaru (embedded)");
                if let Some(def) = kzktdk::font::AppConfig::get_latin() { println!("Global default latin: {}", def); }
                if let Some(def) = kzktdk::font::AppConfig::get_cjk() { println!("Global default cjk: {}", def); }
            }
            FontCmd::Remove { name } => {
                kzktdk::font::FontRegistry::remove(&name)?;
                println!("Removed font '{}'", name);
            }
            FontCmd::SetDefault { name, cjk } => {
                if cjk {
                    kzktdk::font::AppConfig::set_cjk(&name)?;
                    println!("Set global default CJK font to '{}'", name);
                } else {
                    kzktdk::font::AppConfig::set_latin(&name)?;
                    println!("Set global default Latin font to '{}'", name);
                }
            }
            FontCmd::GetDefault => {
                if let Some(def) = kzktdk::font::AppConfig::get_latin() { println!("Global default latin: {}", def); } else { println!("Global default latin: Komika Axis (embedded)"); }
                if let Some(def) = kzktdk::font::AppConfig::get_cjk() { println!("Global default cjk: {}", def); } else { println!("Global default cjk: KosugiMaru (embedded)"); }
            }
        }
    }

    Ok(())
}

struct TranslationContext<'a> {
    chain: &'a ProviderChain,
    rate_limiter: &'a RateLimiter,
    prompt: &'a str,
    prompt_sig: &'a str,
    target_lang: &'a str,
    font_bytes: &'a [u8],
    cjk_font_bytes: Option<&'a [u8]>,
    batch_size: usize,
    cache: Option<&'a TranslationCache>,
    save_metadata: bool,
    metadata_dir: Option<PathBuf>,
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

    // Cache filter
    let mut all_translations = std::collections::HashMap::new();
    let crops_to_translate: Vec<CropItem>;
    if let Some(cache) = ctx.cache {
        // Use primary provider identity for cache key
        let prov_name = ctx.chain.primary.name();
        let model_name = ctx.chain.primary.model_name();
        let (cached, to_trans) = cache.filter_cached(&crops, ctx.target_lang, prov_name, model_name, ctx.prompt_sig);
        if !cached.is_empty() {
            println!("      [Cache Hit] {}/{} bubbles from cache", cached.len(), crops.len());
        }
        all_translations.extend(cached);
        crops_to_translate = to_trans;
    } else {
        crops_to_translate = crops.clone();
    }

    if crops_to_translate.is_empty() {
        println!("      All bubbles served from cache");
    } else {
        // Chunk into batches for LLM translation with fallback+retry+repair
        for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
            let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
            let chunk_trans = translate_with_chain(ctx.chain, &mosaic, ctx.prompt, ctx.target_lang, ctx.rate_limiter).await?;
            // Save to cache
            if let Some(cache) = ctx.cache {
                let prov_name = ctx.chain.primary.name();
                let model_name = ctx.chain.primary.model_name();
                cache.save_batch(&chunk_trans, chunk, ctx.target_lang, prov_name, model_name, ctx.prompt_sig);
            }
            all_translations.extend(chunk_trans);
        }
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

    // Save metadata sidecar if requested
    if ctx.save_metadata {
        let (w, h) = img.dimensions();
        let page_name = input_path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let mut bg_map = std::collections::HashMap::new();
        // sample bg color not critical, leave empty; editor will compute if needed
        let data = metadata::build_page_data(&page_name, w, h, ctx.target_lang, ctx.prompt_sig, &detections, &all_translations, &bg_map);
        let meta_dir = ctx.metadata_dir.clone().unwrap_or_else(|| output_path.parent().unwrap_or(Path::new(".")).to_path_buf());
        std::fs::create_dir_all(&meta_dir)?;
        let meta_path = meta_dir.join(format!("{}.kedit.json", output_path.file_stem().unwrap_or_default().to_string_lossy()));
        metadata::save_page_metadata(&meta_path, &data)?;
        println!("    [Metadata] Saved to {:?}", meta_path);
    }

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
