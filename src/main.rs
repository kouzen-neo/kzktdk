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

  kzktdk metadata export <input> --json page.kedit.json
  kzktdk metadata edit <json> --set \"1=Halo\" --bbox \"1=x1,y1,x2,y2\" --font-size \"1=22\"
  kzktdk metadata render <image> --metadata page.kedit.json -o out.jpg
  kzktdk metadata render --project project.kedit.json --images orig/ -o rendered/ --jobs auto
  kzktdk metadata preview <image> --metadata page.kedit.json --id 1 --text \"Hi\" -o prev.jpg
  kzktdk metadata show <json> [--id 1]           Show bubbles table
  kzktdk metadata pack <folder_rendered/> -o chapter.cbz
  kzktdk font list / import <path> / set-default <name>

Commands:

  translate   Full pipeline: Detect -> Inpaint -> Translate -> Typeset
  metadata    Editor backend: export / edit / render / preview / show / pack
  font        Font registry: import / list / remove / set-default
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
        /// Also detect freetext outside bubbles (requires --ocr)
        #[arg(long, default_value_t = false)]
        translate_free_text: bool,
        /// OCR script for freetext: auto|jp|en|kr|cn
        #[arg(long, default_value = "jp")]
        ocr_script: String,
        /// OCR engine for freetext: none|rapid|manga|tesseract
        #[arg(long, default_value = "none", value_parser = clap::builder::PossibleValuesParser::new(["none", "rapid", "manga", "tesseract", "vision", "local"]))]
        ocr: String,
        /// Custom OCR model path override
        #[arg(long)]
        ocr_model: Option<PathBuf>,
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
        /// OCR engine: none (default), rapid, manga, tesseract — pluggable
        #[arg(long, default_value = "none", value_parser = clap::builder::PossibleValuesParser::new(["none", "rapid", "manga", "tesseract", "vision", "local"]))]
        ocr: String,
        /// Translate freetext outside bubbles (requires --ocr rapid|manga|tesseract)
        #[arg(long, default_value_t = false)]
        translate_free_text: bool,
        /// OCR script: auto|jp|en|kr|cn (default jp = JAPANESE+Latin)
        #[arg(long, default_value = "jp")]
        ocr_script: String,
        /// Translation mode: vision (mosaic), ocr (text-only), auto (ocr fallback vision)
        #[arg(long, default_value = "vision", value_parser = clap::builder::PossibleValuesParser::new(["vision", "ocr", "auto"]))]
        mode: String,
        /// Custom OCR model path override (else auto-download ~/.cache/kzktdk/models/<engine>/)
        #[arg(long)]
        ocr_model: Option<PathBuf>,
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
    /// Render image from metadata JSON without LLM (single or batch via --project)
    Render {
        /// Original image path (for single mode)
        image: Option<PathBuf>,
        /// Metadata JSON file (.kedit.json) (for single mode)
        #[arg(long)]
        metadata: Option<PathBuf>,
        /// Project JSON file (for batch mode: render all pages in project)
        #[arg(long)]
        project: Option<PathBuf>,
        /// Images folder (for batch mode, overrides project image paths)
        #[arg(long)]
        images: Option<PathBuf>,
        /// Output: image file (single) or folder (batch)
        #[arg(short, long)]
        output: PathBuf,
        /// Font path
        #[arg(short, long, default_value = "fonts/Komika Axis.ttf")]
        font: PathBuf,
        /// CJK font path
        #[arg(long, default_value = "fonts/KosugiMaru.ttf")]
        cjk_font: PathBuf,
        /// Jobs for batch render (auto or N)
        #[arg(long, default_value = "auto")]
        jobs: String,
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
        /// Preview override: font family (without saving)
        #[arg(long, value_name = "FontName")]
        font_family: Option<String>,
        /// Preview override: font size
        #[arg(long, value_name = "Size")]
        font_size: Option<f32>,
        /// Preview override: text color R,G,B
        #[arg(long, value_name = "R,G,B")]
        text_color: Option<String>,
        /// Preview override: stroke color R,G,B
        #[arg(long, value_name = "R,G,B")]
        stroke_color: Option<String>,
        /// Preview override: align left|center|right
        #[arg(long, value_name = "Align")]
        align: Option<String>,
        /// Show raw_text instead of translated when available
        #[arg(long)]
        show_raw: bool,
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
        /// Set bg color per bubble: "ID=R,G,B" (can repeat, empty to clear)
        #[arg(long, value_name = "ID=R,G,B")]
        bg_color: Vec<String>,
        /// Set confidence per bubble: "ID=0.99" (can repeat)
        #[arg(long, value_name = "ID=Conf")]
        conf: Vec<String>,
        /// Clear whole style for bubble IDs (can repeat, set style=None)
        #[arg(long, value_name = "ID")]
        clear_style: Vec<String>,
        /// Set raw OCR text: "ID=raw text" (can repeat, empty to clear)
        #[arg(long, value_name = "ID=Text")]
        raw_text: Vec<String>,
        /// Find & replace across all bubbles: "old=new" (can repeat)
        #[arg(long, value_name = "Old=New")]
        replace: Vec<String>,
        /// Apply style to all bubbles: "align=center,bold=true,italic=false,font_size=20,text_color=255,0,0"
        #[arg(long, value_name = "Spec")]
        apply_style: Option<String>,
        /// Set bold per bubble: "ID=true/false" (can repeat, empty to clear)
        #[arg(long, value_name = "ID=Bool")]
        bold: Vec<String>,
        /// Set italic per bubble: "ID=true/false" (can repeat, empty to clear)
        #[arg(long, value_name = "ID=Bool")]
        italic: Vec<String>,
    },
    /// Validate metadata JSON
    Validate {
        /// JSON file to validate
        json: PathBuf,
    },
    /// Show metadata as table (auto-detect PageEditData or Project)
    Show {
        /// JSON file (PageEditData .kedit.json or project.kedit.json)
        json: PathBuf,
        /// Filter by bubble ID (only for PageEditData)
        #[arg(long)]
        id: Option<String>,
    },
    /// Pack rendered folder into CBZ archive
    Pack {
        /// Input folder with rendered images
        input: PathBuf,
        /// Output CBZ file
        #[arg(short, long)]
        output: PathBuf,
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
            translate_free_text,
            ocr_script,
            ocr,
            ocr_model,
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
                    let mut detections = yolo.detect_bubbles(&img)?;
                    println!("Found {} speech bubbles.", detections.len());
                    // freetext
                    let mut ft_boxes: Vec<[u32;4]> = Vec::new();
                    if translate_free_text {
                        if ocr == "none" {
                            eprintln!("[freetext] butuh --ocr rapid|manga|tesseract, fallback bubble-only");
                        } else {
                            let script = kzktdk::ocr::OcrScript::from_key(&ocr_script);
                            let engine = kzktdk::ocr::create_ocr_engine(&ocr, ocr_model.as_deref(), script);
                            if engine.name() != "none" {
                                let bubble_boxes: Vec<[u32;4]> = detections.iter().map(|d| [d.x1,d.y1,d.x2,d.y2]).collect();
                                ft_boxes = kzktdk::preparer::detect_free_text(&img.to_rgb8(), &bubble_boxes, engine.as_ref());
                                println!("Found {} freetext regions.", ft_boxes.len());
                            }
                        }
                    }
                    let mut rgb_img = img.to_rgb8();
                    for (idx, det) in detections.iter().enumerate() {
                        println!("  Bubble #{}: [{}, {}, {}, {}] (conf: {:.2})", idx+1, det.x1, det.y1, det.x2, det.y2, det.conf);
                        draw_rect(&mut rgb_img, det.x1, det.y1, det.x2, det.y2, Rgb([255,0,0]), 2);
                    }
                    for (idx, fb) in ft_boxes.iter().enumerate() {
                        println!("  Freetext #{}: [{},{},{},{}]", idx+1, fb[0],fb[1],fb[2],fb[3]);
                        draw_rect(&mut rgb_img, fb[0],fb[1],fb[2],fb[3], Rgb([0,200,0]), 2);
                    }
                    rgb_img.save(&out_path)?;
                    println!("Saved detection preview to {:?}", out_path);
                    if let Some(json_path) = json {
                        let mut bubbles: Vec<metadata::Bubble> = detections.iter().enumerate().map(|(i,d)| metadata::Bubble {
                            id: (i+1).to_string(), bbox: [d.x1,d.y1,d.x2,d.y2], conf: d.conf, translated: String::new(), bg_color: None, style: None, edited: false, raw_text: None
                        }).collect();
                        for (i, fb) in ft_boxes.iter().enumerate() {
                            bubbles.push(metadata::Bubble { id: format!("ft{}", i+1), bbox: *fb, conf: 0.90, translated: String::new(), bg_color: None, style: None, edited: false, raw_text: None });
                        }
                        let mut data = metadata::PageEditData::new(
                            img_path.file_name().unwrap().to_string_lossy().to_string(),
                            w, h, "English".to_string(), "classic".to_string(),
                            bubbles
                        );
                        if ocr != "none" { data.ocr_engine = Some(ocr.clone()); }
                        metadata::save_page_metadata(&json_path, &data)?;
                        println!("Saved JSON to {:?}", json_path);
                    }
                }
                PreparedInput::Batch { images, _temp_guard: _, is_archive: _, original_name } => {
                    let out_dir = output.unwrap_or_else(|| PathBuf::from("detected"));
                    std::fs::create_dir_all(&out_dir)?;
                    println!("[2/2] Detecting bubbles on {} pages (jobs={})...", images.len(), jobs_num);
                    let mut all_pages = Vec::new();
                    let make_page_with_ft = |p: &PathBuf, dets: Vec<kzktdk::model::yolo::Detection>, w: u32, h: u32, rgb_ref: &RgbImage| -> metadata::PageEditData {
                        let mut ft_boxes: Vec<[u32;4]> = Vec::new();
                        if translate_free_text && ocr != "none" {
                            let script = kzktdk::ocr::OcrScript::from_key(&ocr_script);
                            let engine = kzktdk::ocr::create_ocr_engine(&ocr, ocr_model.as_deref(), script);
                            if engine.name() != "none" {
                                let bboxes: Vec<[u32;4]> = dets.iter().map(|d| [d.x1,d.y1,d.x2,d.y2]).collect();
                                ft_boxes = kzktdk::preparer::detect_free_text(rgb_ref, &bboxes, engine.as_ref());
                            }
                        }
                        let mut bubbles: Vec<metadata::Bubble> = dets.iter().enumerate().map(|(i,d)| metadata::Bubble { id: (i+1).to_string(), bbox: [d.x1,d.y1,d.x2,d.y2], conf: d.conf, translated: String::new(), bg_color: None, style: None, edited: false, raw_text: None }).collect();
                        for (i, fb) in ft_boxes.iter().enumerate() {
                            bubbles.push(metadata::Bubble { id: format!("ft{}", i+1), bbox: *fb, conf: 0.90, translated: String::new(), bg_color: None, style: None, edited: false, raw_text: None });
                        }
                        let mut data = metadata::PageEditData::new(p.file_name().unwrap().to_string_lossy().to_string(), w, h, "English".to_string(), "classic".to_string(), bubbles);
                        if ocr != "none" { data.ocr_engine = Some(ocr.clone()); }
                        data
                    };
                    if jobs_num <= 1 {
                        for (idx, p) in images.iter().enumerate() {
                            let img = image::open(p).with_context(|| format!("Failed to open {:?}", p))?;
                            let (w,h) = (img.width(), img.height());
                            let dets = yolo.detect_bubbles(&img)?;
                            println!("[Page {}/{}] {:?}: {} bubbles", idx+1, images.len(), p.file_name().unwrap(), dets.len());
                            let rgb = img.to_rgb8();
                            let mut rgb_out = rgb.clone();
                            for d in &dets { draw_rect(&mut rgb_out, d.x1, d.y1, d.x2, d.y2, Rgb([255,0,0]), 2); }
                            // also draw ft
                            if translate_free_text && ocr != "none" {
                                let script = kzktdk::ocr::OcrScript::from_key(&ocr_script);
                                let engine = kzktdk::ocr::create_ocr_engine(&ocr, ocr_model.as_deref(), script);
                                if engine.name() != "none" {
                                    let bboxes: Vec<[u32;4]> = dets.iter().map(|d| [d.x1,d.y1,d.x2,d.y2]).collect();
                                    let fts = kzktdk::preparer::detect_free_text(&rgb, &bboxes, engine.as_ref());
                                    for fb in &fts { draw_rect(&mut rgb_out, fb[0],fb[1],fb[2],fb[3], Rgb([0,200,0]), 2); }
                                }
                            }
                            let out = out_dir.join(p.file_name().unwrap());
                            rgb_out.save(&out)?;
                            if json.is_some() {
                                all_pages.push(make_page_with_ft(p, dets, w, h, &rgb));
                            }
                        }
                    } else {
                        // For batch detect, sequential yolo is fine; parallel would need Send. Keep sequential for now with progress.
                        for (idx, p) in images.iter().enumerate() {
                            let img = image::open(p).with_context(|| format!("Failed to open {:?}", p))?;
                            let (w,h) = (img.width(), img.height());
                            let dets = yolo.detect_bubbles(&img)?;
                            println!("[Page {}/{}] {:?}: {} bubbles", idx+1, images.len(), p.file_name().unwrap(), dets.len());
                            let rgb = img.to_rgb8();
                            let mut rgb_out = rgb.clone();
                            for d in &dets { draw_rect(&mut rgb_out, d.x1, d.y1, d.x2, d.y2, Rgb([255,0,0]), 2); }
                            if translate_free_text && ocr != "none" {
                                let script = kzktdk::ocr::OcrScript::from_key(&ocr_script);
                                let engine = kzktdk::ocr::create_ocr_engine(&ocr, ocr_model.as_deref(), script);
                                if engine.name() != "none" {
                                    let bboxes: Vec<[u32;4]> = dets.iter().map(|d| [d.x1,d.y1,d.x2,d.y2]).collect();
                                    let fts = kzktdk::preparer::detect_free_text(&rgb, &bboxes, engine.as_ref());
                                    for fb in &fts { draw_rect(&mut rgb_out, fb[0],fb[1],fb[2],fb[3], Rgb([0,200,0]), 2); }
                                }
                            }
                            let out = out_dir.join(p.file_name().unwrap());
                            rgb_out.save(&out)?;
                            if json.is_some() {
                                all_pages.push(make_page_with_ft(p, dets, w, h, &rgb));
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
            ocr,
            translate_free_text,
            ocr_script,
            mode,
            ocr_model,
        } => {
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
                        translate_free_text: translate_free_text.clone(),
                        ocr: ocr.clone(),
                        ocr_script: ocr_script.clone(),
                        mode: mode.clone(),
                        ocr_model: ocr_model.clone(),
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
                            translate_free_text: translate_free_text.clone(),
                            ocr: ocr.clone(),
                            ocr_script: ocr_script.clone(),
                            mode: mode.clone(),
                            ocr_model: ocr_model.clone(),
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
                        let translate_free_text_c = translate_free_text.clone();
                        let ocr_c = ocr.clone();
                        let ocr_script_c = ocr_script.clone();
                        let mode_c = mode.clone();
                        let ocr_model_c = ocr_model.clone();

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
                            let translate_free_text_c2 = translate_free_text_c.clone();
                            let ocr_c2 = ocr_c.clone();
                            let ocr_script_c2 = ocr_script_c.clone();
                            let mode_c2 = mode_c.clone();
                            let ocr_model_c2 = ocr_model_c.clone();
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
                                    translate_free_text: translate_free_text_c2,
                                    ocr: ocr_c2,
                                    ocr_script: ocr_script_c2,
                                    mode: mode_c2,
                                    ocr_model: ocr_model_c2,
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
                        dets.iter().enumerate().map(|(i,d)| metadata::Bubble { id: (i+1).to_string(), bbox: [d.x1,d.y1,d.x2,d.y2], conf: d.conf, translated: String::new(), bg_color: None, style: None, edited: false, raw_text: None }).collect()
                    ));
                }
                // Single page -> object, multi -> array for backwards compat
                let v = if pages.len()==1 { serde_json::to_value(&pages[0]).unwrap() } else { serde_json::to_value(&pages).unwrap() };
                std::fs::create_dir_all(json.parent().unwrap_or(Path::new(".")))?;
                std::fs::write(&json, serde_json::to_string_pretty(&v).unwrap())?;
                println!("Exported {} pages to {:?}", pages.len(), json);
            }
            MetadataCmd::Render { image, metadata, project, images, output, font, cjk_font, jobs } => {
                if let Some(proj_path) = project {
                    // Batch render via project.kedit.json
                    let proj = metadata::load_project(&proj_path).with_context(|| format!("Failed to load project {:?}", proj_path))?;
                    let jobs_num = parse_jobs(&jobs);
                    let proj_dir = proj_path.parent().unwrap_or(Path::new(".")).to_path_buf();
                    let images_dir = images.clone();
                    std::fs::create_dir_all(&output)?;
                    println!("[Batch Render] {} pages jobs={} -> {:?}", proj.pages.len(), jobs_num, output);
                    let font_bytes_global = if font.exists() { std::fs::read(font)? } else if let Some(p) = find_file_in_candidates("fonts/Komika Axis.ttf") { std::fs::read(p)? } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() };
                    let cjk_bytes_global = if cjk_font.exists() { std::fs::read(&cjk_font).ok() } else if let Some(p) = find_file_in_candidates("fonts/KosugiMaru.ttf") { std::fs::read(p).ok() } else { None };
                    for (idx, page_meta_str) in proj.pages.iter().enumerate() {
                        let meta_path = {
                            let p = PathBuf::from(page_meta_str);
                            if p.exists() { p } else { proj_dir.join(Path::new(page_meta_str).file_name().unwrap_or_default()) }
                        };
                        let data = metadata::load_page_metadata(&meta_path).with_context(|| format!("Failed to load {:?}", meta_path))?;
                        // Resolve image path: --images folder overrides, else derive from meta page name
                        let img_path = if let Some(ref img_dir) = images_dir {
                            let cand1 = img_dir.join(&data.page);
                            if cand1.exists() { cand1 } else {
                                // try natural sort match by index
                                let entries: Vec<PathBuf> = std::fs::read_dir(img_dir).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|e| e.eq_ignore_ascii_case("jpg")||e.eq_ignore_ascii_case("png")||e.eq_ignore_ascii_case("webp")).unwrap_or(false)).collect()).unwrap_or_default();
                                if idx < entries.len() { entries[idx].clone() } else { cand1 }
                            }
                        } else {
                            // try sibling of meta file
                            let cand = meta_path.with_file_name(&data.page);
                            if cand.exists() { cand } else { PathBuf::from(&data.page) }
                        };
                        if !img_path.exists() {
                            eprintln!("[{}/{}] Skip {}: image not found {:?}", idx+1, proj.pages.len(), data.page, img_path);
                            continue;
                        }
                        let img = image::open(&img_path).with_context(|| format!("Failed to open {:?}", img_path))?;
                        let mut rgb = img.to_rgb8();
                        let dets: Vec<kzktdk::model::yolo::Detection> = data.bubbles.iter().filter(|b| b.translated.to_uppercase() != "SKIP" && !b.translated.trim().is_empty()).map(|b| kzktdk::model::yolo::Detection { x1: b.bbox[0], y1: b.bbox[1], x2: b.bbox[2], y2: b.bbox[3], conf: b.conf }).collect();
                        if !dets.is_empty() { inpaint_image(&mut rgb, &dets)?; }
                        let global_typesetter = Typesetter::new(&font_bytes_global, cjk_bytes_global.as_deref())?;
                        for b in &data.bubbles {
                            if b.translated.to_uppercase() == "SKIP" || b.translated.trim().is_empty() { continue; }
                            let det = kzktdk::model::yolo::Detection { x1: b.bbox[0], y1: b.bbox[1], x2: b.bbox[2], y2: b.bbox[3], conf: b.conf };
                            if let Some(style) = &b.style {
                                if let Some(ref fname) = style.font_family {
                                    let mut custom_bytes: Option<Vec<u8>> = None;
                                    if kzktdk::font::FontRegistry::resolve(fname).is_ok() {
                                        let list = kzktdk::font::FontRegistry::list();
                                        if let Some(info) = list.iter().find(|f| &f.name == fname) { custom_bytes = std::fs::read(&info.path).ok(); }
                                    } else if Path::new(fname).exists() { custom_bytes = std::fs::read(fname).ok(); }
                                    if let Some(bytes) = custom_bytes {
                                        if let Ok(ts) = Typesetter::new(&bytes, cjk_bytes_global.as_deref()) {
                                            ts.render_bubble_text_with_style(&mut rgb, &det, &b.translated, Some(&data.target_lang), None, Some(style));
                                            continue;
                                        }
                                    }
                                }
                            }
                            global_typesetter.render_bubble_text_with_style(&mut rgb, &det, &b.translated, Some(&data.target_lang), None, b.style.as_ref());
                        }
                        let out_path = output.join(Path::new(&data.page).file_name().unwrap_or_else(|| std::ffi::OsStr::new(&data.page)));
                        rgb.save(&out_path)?;
                        println!("[{}/{}] Rendered {:?} -> {:?}", idx+1, proj.pages.len(), img_path.file_name().unwrap(), out_path);
                    }
                    println!("Batch render complete -> {:?}", output);
                } else {
                    // Single render
                    let img_path = image.clone().context("Missing <IMAGE> or --project for render")?;
                    let meta_path = metadata.clone().context("Missing --metadata for single render")?;
                    let data = metadata::load_page_metadata(&meta_path)?;
                    let img = image::open(&img_path).with_context(|| format!("Failed to open {:?}", img_path))?;
                    let mut rgb = img.to_rgb8();
                    let dets: Vec<kzktdk::model::yolo::Detection> = data.bubbles.iter().filter(|b| b.translated.to_uppercase() != "SKIP" && !b.translated.trim().is_empty()).map(|b| kzktdk::model::yolo::Detection { x1: b.bbox[0], y1: b.bbox[1], x2: b.bbox[2], y2: b.bbox[3], conf: b.conf }).collect();
                    if !dets.is_empty() { inpaint_image(&mut rgb, &dets)?; }
                    let font_bytes = if font.exists() { std::fs::read(&font)? } else {
                        let fstr = font.to_string_lossy().to_string();
                        if !fstr.is_empty() && kzktdk::font::FontRegistry::resolve(&fstr).is_ok() {
                            let list = kzktdk::font::FontRegistry::list();
                            if let Some(info) = list.iter().find(|f| f.name == fstr) { std::fs::read(&info.path).unwrap_or_else(|_| include_bytes!("../fonts/Komika Axis.ttf").to_vec()) } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() }
                        } else if let Some(p) = find_file_in_candidates("fonts/Komika Axis.ttf") { std::fs::read(p)? } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() }
                    };
                    let cjk_bytes = if cjk_font.exists() { std::fs::read(&cjk_font).ok() } else if let Some(p) = find_file_in_candidates("fonts/KosugiMaru.ttf") { std::fs::read(p).ok() } else { None };
                    let global_typesetter = Typesetter::new(&font_bytes, cjk_bytes.as_deref())?;
                    for b in &data.bubbles {
                        if b.translated.to_uppercase() == "SKIP" || b.translated.trim().is_empty() { continue; }
                        let det = kzktdk::model::yolo::Detection { x1: b.bbox[0], y1: b.bbox[1], x2: b.bbox[2], y2: b.bbox[3], conf: b.conf };
                        if let Some(style) = &b.style {
                            if let Some(ref fname) = style.font_family {
                                let mut custom_bytes: Option<Vec<u8>> = None;
                                if kzktdk::font::FontRegistry::resolve(fname).is_ok() {
                                    let list = kzktdk::font::FontRegistry::list();
                                    if let Some(info) = list.iter().find(|f| &f.name == fname) { custom_bytes = std::fs::read(&info.path).ok(); }
                                } else if Path::new(fname).exists() { custom_bytes = std::fs::read(fname).ok(); }
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
                    if let Some(parent) = output.parent() { std::fs::create_dir_all(parent)?; }
                    rgb.save(&output)?;
                    println!("Rendered {:?} -> {:?}", img_path, output);
                }
            }
            MetadataCmd::Edit { json, set, bbox, add, delete, font_family, font_size, text_color, stroke_color, align, edited, bg_color, conf, clear_style, raw_text, replace, apply_style, bold, italic } => {
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
                            if parts[0] >= parts[2] || parts[1] >= parts[3] {
                                eprintln!("Invalid bbox for {}: x1<x2 and y1<y2 required", id);
                                continue;
                            }
                            if parts[2] > data.width || parts[3] > data.height {
                                eprintln!("Invalid bbox for {}: x2<=width({}) y2<=height({}) required, got {:?}", id, data.width, data.height, parts);
                                continue;
                            }
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
                    if let Some((id, rest)) = s.split_once('=') {
                        let (bbox_str, txt) = if let Some(eq_pos) = rest.find('=') {
                            let candidate_bbox = &rest[..eq_pos];
                            let comma_cnt = candidate_bbox.matches(',').count();
                            if comma_cnt == 3 { (&rest[..eq_pos], &rest[eq_pos+1..]) } else { (rest, "") }
                        } else { (rest, "") };
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
                        if parts[2] > data.width || parts[3] > data.height {
                            eprintln!("Invalid bbox for add {}: out of bounds {}x{}", id, data.width, data.height);
                            continue;
                        }
                        data.bubbles.push(metadata::Bubble { id: id.to_string(), bbox: [parts[0],parts[1],parts[2],parts[3]], conf: 1.0, translated: txt.to_string(), bg_color: None, style: None, edited: false, raw_text: None });
                        println!("Added bubble {} bbox {:?} text \"{}\"", id, [parts[0],parts[1],parts[2],parts[3]], txt);
                    } else {
                        eprintln!("Invalid --add format: expected ID=x1,y1,x2,y2[=text] got '{}'", s);
                    }
                }
                for id in delete {
                    let before = data.bubbles.len();
                    data.bubbles.retain(|b| b.id != id);
                    if data.bubbles.len() < before { println!("Deleted bubble {}", id); } else { eprintln!("Bubble {} not found for delete", id); }
                }
                for s in font_family {
                    if let Some((id, font)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if font.is_empty() { if let Some(style) = b.style.as_mut() { style.font_family = None; } println!("Cleared font for {}", id); } else { let style = b.style.get_or_insert_with(metadata::BubbleStyle::default); style.font_family = Some(font.to_string()); println!("Set font {} = {}", id, font); }
                        } else { eprintln!("Bubble {} not found for font_family", id); }
                    }
                }
                for s in font_size {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() { if let Some(style) = b.style.as_mut() { style.font_size = None; } println!("Cleared font_size for {}", id); } else if let Ok(f) = val.parse::<f32>() { let style = b.style.get_or_insert_with(metadata::BubbleStyle::default); style.font_size = Some(f); println!("Set font_size {} = {}", id, f); } else { eprintln!("Invalid font_size for {}: {}", id, val); }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in text_color {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() { if let Some(style) = b.style.as_mut() { style.text_color = None; } println!("Cleared text_color for {}", id); } else { let parts: Vec<u8> = val.split(',').filter_map(|v| v.trim().parse().ok()).collect(); if parts.len()==3 { let style = b.style.get_or_insert_with(metadata::BubbleStyle::default); style.text_color = Some([parts[0],parts[1],parts[2]]); println!("Set text_color {} = {:?}", id, style.text_color); } else { eprintln!("Invalid text_color for {}: expected R,G,B", id); } }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in stroke_color {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() { if let Some(style) = b.style.as_mut() { style.stroke_color = None; } println!("Cleared stroke_color for {}", id); } else { let parts: Vec<u8> = val.split(',').filter_map(|v| v.trim().parse().ok()).collect(); if parts.len()==3 { let style = b.style.get_or_insert_with(metadata::BubbleStyle::default); style.stroke_color = Some([parts[0],parts[1],parts[2]]); println!("Set stroke_color {} = {:?}", id, style.stroke_color); } else { eprintln!("Invalid stroke_color for {}: expected R,G,B", id); } }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in align {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() { if let Some(style) = b.style.as_mut() { style.align = None; } println!("Cleared align for {}", id); } else { let style = b.style.get_or_insert_with(metadata::BubbleStyle::default); style.align = Some(val.to_string()); println!("Set align {} = {}", id, val); }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in edited {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if let Ok(v) = val.parse::<bool>() { b.edited = v; println!("Set edited {} = {}", id, v); } else { eprintln!("Invalid edited for {}: expected true/false", id); }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in bg_color {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() { b.bg_color = None; println!("Cleared bg_color for {}", id); } else { let parts: Vec<u8> = val.split(',').filter_map(|v| v.trim().parse().ok()).collect(); if parts.len()==3 { b.bg_color = Some([parts[0],parts[1],parts[2]]); println!("Set bg_color {} = {:?}", id, b.bg_color); } else { eprintln!("Invalid bg_color for {}: expected R,G,B", id); } }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in conf {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if let Ok(v) = val.parse::<f32>() { b.conf = v.clamp(0.0, 1.0); println!("Set conf {} = {}", id, b.conf); } else { eprintln!("Invalid conf for {}: {}", id, val); }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for id in clear_style {
                    if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) { b.style = None; println!("Cleared style for {}", id); } else { eprintln!("Bubble {} not found for clear_style", id); }
                }
                for s in raw_text {
                    if let Some((id, txt)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if txt.is_empty() { b.raw_text = None; println!("Cleared raw_text for {}", id); } else { b.raw_text = Some(txt.to_string()); println!("Set raw_text {} = \"{}\"", id, txt); }
                        } else { eprintln!("Bubble {} not found for raw_text", id); }
                    }
                }
                for rep in replace {
                    if let Some((old, new)) = rep.split_once('=') {
                        let mut cnt = 0;
                        for b in data.bubbles.iter_mut() {
                            if b.translated.contains(old) {
                                b.translated = b.translated.replace(old, new);
                                b.edited = true;
                                cnt += 1;
                            }
                        }
                        println!("Replace \"{}\" -> \"{}\" in {} bubbles", old, new, cnt);
                    } else { eprintln!("Invalid --replace format: expected Old=New got '{}'", rep); }
                }
                if let Some(spec) = apply_style {
                    // Spec: comma-separated key=value e.g. align=center,bold=true,font_size=20
                    let pairs: Vec<(&str,&str)> = spec.split(',').filter_map(|p| p.split_once('=')).collect();
                    let mut cnt = 0;
                    for b in data.bubbles.iter_mut() {
                        let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                        for (k,v) in &pairs {
                            match k.trim().to_lowercase().as_str() {
                                "align" => { if v.is_empty() { style.align=None } else { style.align=Some(v.to_string()) } }
                                "font_size" | "fontsize" | "size" => { if v.is_empty() { style.font_size=None } else if let Ok(f)=v.parse::<f32>() { style.font_size=Some(f) } }
                                "font_family" | "font" => { if v.is_empty() { style.font_family=None } else { style.font_family=Some(v.to_string()) } }
                                "text_color" | "tc" => { if v.is_empty() { style.text_color=None } else { let p: Vec<u8>=v.split(',').filter_map(|x| x.trim().parse().ok()).collect(); if p.len()==3 { style.text_color=Some([p[0],p[1],p[2]]) } } }
                                "stroke_color" | "sc" => { if v.is_empty() { style.stroke_color=None } else { let p: Vec<u8>=v.split(',').filter_map(|x| x.trim().parse().ok()).collect(); if p.len()==3 { style.stroke_color=Some([p[0],p[1],p[2]]) } } }
                                "bold" | "is_bold" => { if v.is_empty() { style.is_bold=None } else if let Ok(bv)=v.parse::<bool>() { style.is_bold=Some(bv) } }
                                "italic" | "is_italic" => { if v.is_empty() { style.is_italic=None } else if let Ok(bv)=v.parse::<bool>() { style.is_italic=Some(bv) } }
                                _ => {}
                            }
                        }
                        cnt += 1;
                    }
                    println!("Apply style to {} bubbles: {}", cnt, spec);
                }
                for s in bold {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() { if let Some(style)=b.style.as_mut() { style.is_bold=None } println!("Cleared bold for {}", id); } else if let Ok(v)=val.parse::<bool>() { let style=b.style.get_or_insert_with(metadata::BubbleStyle::default); style.is_bold=Some(v); println!("Set bold {} = {}", id, v); } else { eprintln!("Invalid bold for {}: {}", id, val); }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                for s in italic {
                    if let Some((id, val)) = s.split_once('=') {
                        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            if val.is_empty() { if let Some(style)=b.style.as_mut() { style.is_italic=None } println!("Cleared italic for {}", id); } else if let Ok(v)=val.parse::<bool>() { let style=b.style.get_or_insert_with(metadata::BubbleStyle::default); style.is_italic=Some(v); println!("Set italic {} = {}", id, v); } else { eprintln!("Invalid italic for {}: {}", id, val); }
                        } else { eprintln!("Bubble {} not found", id); }
                    }
                }
                metadata::save_page_metadata(&json, &data)?;
                println!("Saved edited metadata to {:?}", json);
            }
            MetadataCmd::Validate { json } => {
                let content = std::fs::read_to_string(&json).with_context(|| format!("Failed to read {:?}", json))?;
                // Try project first
                if let Ok(proj) = serde_json::from_str::<metadata::Project>(&content) {
                    if proj.version == 1 && !proj.pages.is_empty() {
                        println!("Valid Project v{}: {} pages", proj.version, proj.pages.len());
                        for (i, p) in proj.pages.iter().enumerate() { println!("  [{:02}] {}", i+1, p); }
                        return Ok(());
                    }
                }
                let data: metadata::PageEditData = serde_json::from_str(&content).with_context(|| format!("Failed to parse PageEditData {:?}", json))?;
                println!("Valid PageEditData v{}: {} bubbles, page={} ({}x{}) target_lang={} prompt_sig={}", data.version, data.bubbles.len(), data.page, data.width, data.height, data.target_lang, data.prompt_sig);
                let mut seen = std::collections::HashSet::new();
                for b in &data.bubbles {
                    if !seen.insert(&b.id) { eprintln!("  [!] Duplicate id {}", b.id); }
                    if b.bbox[0]>=b.bbox[2] || b.bbox[1]>=b.bbox[3] { eprintln!("  [!] Bubble {} invalid bbox {:?}", b.id, b.bbox); }
                    if b.bbox[2] > data.width || b.bbox[3] > data.height { eprintln!("  [!] Bubble {} bbox out of bounds {:?} vs {}x{}", b.id, b.bbox, data.width, data.height); }
                }
            }
            MetadataCmd::Preview { image, metadata, id, text, output, font, cjk_font, font_family, font_size, text_color, stroke_color, align, show_raw } => {
                let data = metadata::load_page_metadata(&metadata)?;
                let bubble = data.bubbles.iter().find(|b| b.id == id).cloned().context(format!("Bubble {} not found", id))?;
                let display_text = if show_raw { bubble.raw_text.clone().unwrap_or(text.clone()) } else { text.clone() };
                let mut rgb = image::open(&image).with_context(|| format!("Failed to open {:?}", image))?.to_rgb8();
                let det = kzktdk::model::yolo::Detection { x1: bubble.bbox[0], y1: bubble.bbox[1], x2: bubble.bbox[2], y2: bubble.bbox[3], conf: bubble.conf };
                inpaint_image(&mut rgb, &[det.clone()])?;
                let font_bytes = if font.exists() { std::fs::read(&font)? } else if let Some(p) = find_file_in_candidates("fonts/Komika Axis.ttf") { std::fs::read(p)? } else { include_bytes!("../fonts/Komika Axis.ttf").to_vec() };
                let cjk_bytes = if cjk_font.exists() { std::fs::read(&cjk_font).ok() } else if let Some(p) = find_file_in_candidates("fonts/KosugiMaru.ttf") { std::fs::read(p).ok() } else { None };
                let mut preview_style = bubble.style.clone().unwrap_or_default();
                let has_override = font_family.is_some() || font_size.is_some() || text_color.is_some() || stroke_color.is_some() || align.is_some();
                // Apply overrides
                if let Some(ff) = font_family { preview_style.font_family = Some(ff); }
                if let Some(fs) = font_size { preview_style.font_size = Some(fs); }
                if let Some(tc) = text_color {
                    let parts: Vec<u8> = tc.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                    if parts.len()==3 { preview_style.text_color = Some([parts[0],parts[1],parts[2]]); }
                }
                if let Some(sc) = stroke_color {
                    let parts: Vec<u8> = sc.split(',').filter_map(|v| v.trim().parse().ok()).collect();
                    if parts.len()==3 { preview_style.stroke_color = Some([parts[0],parts[1],parts[2]]); }
                }
                if let Some(al) = align { preview_style.align = Some(al); }
                let style_opt: Option<metadata::BubbleStyle> = if has_override || bubble.style.is_some() { Some(preview_style.clone()) } else { None };
                // Resolve custom font for preview if needed
                let custom_bytes: Option<Vec<u8>> = style_opt.as_ref().and_then(|s| s.font_family.as_ref()).and_then(|fname| kzktdk::font::FontRegistry::list().iter().find(|f| &f.name==fname).and_then(|info| std::fs::read(&info.path).ok()));
                let typesetter = if let Some(ref bytes) = custom_bytes {
                    Typesetter::new(bytes, cjk_bytes.as_deref()).unwrap_or_else(|_| Typesetter::new(&font_bytes, cjk_bytes.as_deref()).unwrap())
                } else {
                    Typesetter::new(&font_bytes, cjk_bytes.as_deref())?
                };
                typesetter.render_bubble_text_with_style(&mut rgb, &det, &display_text, Some(&data.target_lang), None, style_opt.as_ref());
                if let Some(parent) = output.parent() { std::fs::create_dir_all(parent)?; }
                rgb.save(&output)?;
                println!("Preview bubble {} -> {:?}", id, output);
                println!("Preview text: \"{}\" (font_size {:?}, align {:?}, font_family {:?}) show_raw={}", display_text, style_opt.as_ref().and_then(|s| s.font_size), style_opt.as_ref().and_then(|s| s.align.as_ref()), style_opt.as_ref().and_then(|s| s.font_family.as_ref()), show_raw);
            }
            MetadataCmd::Show { json, id } => {
                let content = std::fs::read_to_string(&json).with_context(|| format!("Failed to read {:?}", json))?;
                if let Ok(proj) = serde_json::from_str::<metadata::Project>(&content) {
                    if proj.version == 1 && !proj.pages.is_empty() {
                        println!("Project v{}: {} pages target_lang={:?}", proj.version, proj.pages.len(), proj.target_lang);
                        for (i, p) in proj.pages.iter().enumerate() {
                            let meta_path = {
                                let pb = PathBuf::from(p);
                                if pb.exists() { pb } else { json.parent().unwrap_or(Path::new(".")).join(Path::new(p).file_name().unwrap_or_default()) }
                            };
                            let bubbles = metadata::load_page_metadata(&meta_path).map(|d| d.bubbles.len()).unwrap_or(0);
                            println!("  [{:02}] {} ({} bubbles)", i+1, p, bubbles);
                        }
                        return Ok(());
                    }
                }
                let data = metadata::load_page_metadata(&json)?;
                println!("PageEditData v{}: {} ({}x{}) lang={} sig={} ocr={:?} — {} bubbles", data.version, data.page, data.width, data.height, data.target_lang, data.prompt_sig, data.ocr_engine, data.bubbles.len());
                println!("{:<4} {:<18} {:<5} {:<6} {:<22} {:<12} {}", "ID", "BBOX", "CONF", "EDIT", "STYLE", "RAW", "TRANSLATED");
                println!("{}", "-".repeat(130));
                for b in &data.bubbles {
                    if let Some(ref filter) = id { if &b.id != filter { continue; } }
                    let style_str = if let Some(s) = &b.style {
                        let mut parts = Vec::new();
                        if let Some(f) = &s.font_family { parts.push(format!("font={}", f)); }
                        if let Some(fz) = s.font_size { parts.push(format!("sz={}", fz)); }
                        if let Some(c) = s.text_color { parts.push(format!("tc={},{},{}", c[0],c[1],c[2])); }
                        if let Some(c) = s.stroke_color { parts.push(format!("sc={},{},{}", c[0],c[1],c[2])); }
                        if let Some(a) = &s.align { parts.push(format!("al={}", a)); }
                        if let Some(v) = s.is_bold { parts.push(format!("b={}", v)); }
                        if let Some(v) = s.is_italic { parts.push(format!("i={}", v)); }
                        parts.join(",")
                    } else { "-".to_string() };
                    let t: String = {
                        let ch: Vec<char> = b.translated.chars().collect();
                        if ch.len()>30 { ch[..30].iter().collect::<String>() + "..." } else { b.translated.clone() }
                    };
                    let raw = if let Some(r) = &b.raw_text {
                        let ch: Vec<char> = r.chars().collect();
                        if ch.len()>10 { ch[..10].iter().collect::<String>() + "..." } else { r.clone() }
                    } else { "-".to_string() };
                    let bbox = format!("[{},{},{},{}]", b.bbox[0],b.bbox[1],b.bbox[2],b.bbox[3]);
                    println!("{:<4} {:<18} {:<5.2} {:<6} {:<22} {:<12} {}", b.id, bbox, b.conf, b.edited, style_str, raw.replace('\n'," "), t.replace('\n'," "));
                }
            }
            MetadataCmd::Pack { input, output } => {
                if !input.is_dir() { bail!("Pack input must be a folder: {:?}", input); }
                let mut files: Vec<PathBuf> = std::fs::read_dir(&input)?.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file() && p.extension().map(|e| e.eq_ignore_ascii_case("jpg")||e.eq_ignore_ascii_case("png")||e.eq_ignore_ascii_case("jpeg")||e.eq_ignore_ascii_case("webp")).unwrap_or(false)).collect();
                if files.is_empty() { bail!("No images found in {:?}", input); }
                // natural sort
                files.sort_by(|a,b| natord::compare(&a.file_name().unwrap().to_string_lossy(), &b.file_name().unwrap().to_string_lossy()));
                if let Some(parent) = output.parent() { std::fs::create_dir_all(parent)?; }
                archive::create_cbz(&files, &output)?;
                println!("Packed {} images -> {:?}", files.len(), output);
                for (i,f) in files.iter().enumerate() { println!("  [{:02}] {}", i+1, f.file_name().unwrap().to_string_lossy()); }
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
    translate_free_text: bool,
    ocr: String,
    ocr_script: String,
    mode: String,
    ocr_model: Option<PathBuf>,
}

async fn translate_page(
    input_path: &Path,
    output_path: &Path,
    yolo: &mut YoloModel,
    ctx: &TranslationContext<'_>,
) -> Result<()> {
    let img =
        image::open(input_path).with_context(|| format!("Failed to open {:?}", input_path))?;
    let mut detections = yolo.detect_bubbles(&img)?;

    if detections.is_empty() && !ctx.translate_free_text {
        println!("    [Page] No dialogue bubbles detected. Copying original.");
        img.save(output_path)?;
        return Ok(());
    }
    println!("    [Page] Found {} bubbles.", detections.len());
    let orig_rgb = img.to_rgb8();
    let (w_img, h_img) = img.dimensions();

    // --- Freetext detection ---
    let mut ft_boxes: Vec<[u32; 4]> = Vec::new();
    if ctx.translate_free_text {
        if ctx.ocr == "none" {
            eprintln!("[freetext] butuh --ocr rapid|manga|tesseract, fallback bubble-only");
        } else {
            let script = kzktdk::ocr::OcrScript::from_key(&ctx.ocr_script);
            let engine = kzktdk::ocr::create_ocr_engine(&ctx.ocr, ctx.ocr_model.as_deref(), script);
            if engine.name() == "none" {
                eprintln!("[freetext] engine none, skip freetext");
            } else {
                let bubble_boxes: Vec<[u32; 4]> = detections.iter().map(|d| [d.x1, d.y1, d.x2, d.y2]).collect();
                let detected = kzktdk::preparer::detect_free_text(&orig_rgb, &bubble_boxes, engine.as_ref());
                if detected.is_empty() {
                    println!("    [Freetext] 0 regions");
                } else {
                    println!("    [Freetext] {} regions: {:?}", detected.len(), detected);
                    ft_boxes = detected;
                }
            }
        }
    }

    // Build combined detections for rendering/inpaint: bubbles + freetext
    let mut combined_dets: Vec<kzktdk::model::yolo::Detection> = detections.clone();
    let mut ft_ids: Vec<String> = Vec::new();
    for (idx, fb) in ft_boxes.iter().enumerate() {
        let fid = format!("ft{}", idx + 1);
        ft_ids.push(fid);
        combined_dets.push(kzktdk::model::yolo::Detection { x1: fb[0], y1: fb[1], x2: fb[2], y2: fb[3], conf: 0.90 });
    }
    if detections.is_empty() && combined_dets.is_empty() {
        println!("    [Page] No bubbles nor freetext, copying original.");
        img.save(output_path)?;
        return Ok(());
    }

    // Build crops: bubbles + ft
    let mut crops: Vec<CropItem> = Vec::new();
    for (i, det) in detections.iter().enumerate() {
        let w = det.width();
        let h = det.height();
        if w == 0 || h == 0 { continue; }
        println!("      Bubble #{}: [{}, {}, {}, {}] ({}x{})", i+1, det.x1, det.y1, det.x2, det.y2, w, h);
        let mut crop = RgbImage::new(w, h);
        for cy in 0..h { for cx in 0..w { crop.put_pixel(cx, cy, *orig_rgb.get_pixel(det.x1+cx, det.y1+cy)); } }
        crops.push(CropItem { id: (i+1).to_string(), image: crop });
    }
    for (i, fb) in ft_boxes.iter().enumerate() {
        let pad = kzktdk::preparer::freetext_pad(*fb, w_img, h_img);
        let w = (pad[2]-pad[0]).max(1);
        let h = (pad[3]-pad[1]).max(1);
        println!("      Freetext #{}: {:?} padded {:?} ({}x{})", i+1, fb, pad, w, h);
        let mut crop = RgbImage::new(w, h);
        for cy in 0..h { for cx in 0..w { crop.put_pixel(cx, cy, *orig_rgb.get_pixel(pad[0]+cx, pad[1]+cy)); } }
        crops.push(CropItem { id: format!("ft{}", i+1), image: crop });
    }

    // --- OCR raw_text gathering (for metadata) ---
    let mut raw_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if ctx.ocr != "none" {
        let script = kzktdk::ocr::OcrScript::from_key(&ctx.ocr_script);
        let engine = kzktdk::ocr::create_ocr_engine(&ctx.ocr, ctx.ocr_model.as_deref(), script);
        if engine.name() != "none" {
            // recognize per crop (bubble + ft)
            for c in &crops {
                // find bbox for this id
                let bbox_opt = if c.id.starts_with("ft") {
                    ft_boxes.get(c.id[2..].parse::<usize>().unwrap_or(1)-1).copied()
                } else {
                    c.id.parse::<usize>().ok().and_then(|idx| detections.get(idx-1)).map(|d| [d.x1,d.y1,d.x2,d.y2])
                };
                if let Some(bbox) = bbox_opt {
                    if let Some(txt) = engine.recognize(&orig_rgb, bbox) {
                        if !txt.trim().is_empty() { raw_map.insert(c.id.clone(), txt); }
                    }
                }
            }
            if !raw_map.is_empty() { println!("    [OCR] raw_text {} entries", raw_map.len()); }
        }
    }

    // --- Translation branching ---
    let mut all_translations: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    // Cache filter first
    let crops_to_translate: Vec<CropItem>;
    if let Some(cache) = ctx.cache {
        let prov_name = ctx.chain.primary.name();
        let model_name = ctx.chain.primary.model_name();
        let (cached, to_trans) = cache.filter_cached(&crops, ctx.target_lang, prov_name, model_name, ctx.prompt_sig);
        if !cached.is_empty() { println!("      [Cache Hit] {}/{} from cache", cached.len(), crops.len()); }
        all_translations.extend(cached);
        crops_to_translate = to_trans;
    } else {
        crops_to_translate = crops.clone();
    }

    // Normalize ft ids to lowercase (LLM may return FT1)
    fn norm_map(mut m: std::collections::HashMap<String,String>) -> std::collections::HashMap<String,String> {
        let mut out = std::collections::HashMap::new();
        for (k,v) in m.drain() {
            let nk = if k.to_lowercase().starts_with("ft") { k.to_lowercase() } else { k };
            out.insert(nk, v);
        }
        out
    }
    // Helper for text-only translation
    async fn translate_ocr_json(chain: &ProviderChain, limiter: &RateLimiter, json_input: &str, prompt: &str, target_lang: &str) -> Result<std::collections::HashMap<String,String>> {
        let full_prompt = format!("{}\n\nInput JSON (id -> raw Japanese text):\n{}\n\nTranslate each value to {} and return JSON mapping id->translation.", prompt, json_input, target_lang);
        for prov in chain.all_providers() {
            println!("  Translating OCR JSON with {} ({})...", prov.name(), prov.model_name());
            let raw_res = limiter.execute_with_retry(|| prov.translate_text(json_input, &full_prompt)).await;
            match raw_res {
                Ok(raw) => {
                    if let Ok(map) = kzktdk::translation::parse_translation_json(&raw) {
                        if !map.is_empty() { return Ok(norm_map(map)); }
                    }
                    println!("  [Failover] {} unparseable OCR json", prov.name());
                }
                Err(e) => { println!("  [Failover] {} failed ocr json: {}", prov.name(), e); }
            }
        }
        bail!("All providers failed OCR JSON translation")
    }

    if crops_to_translate.is_empty() {
        println!("      All served from cache");
    } else {
        let use_ocr_path = ctx.mode == "ocr" || ctx.mode == "auto";
        let ocr_available = !raw_map.is_empty() || ctx.ocr != "none";
        if ctx.mode == "ocr" && !ocr_available {
            eprintln!("[mode ocr] no OCR results, fallback vision");
        }
        if use_ocr_path && ctx.mode == "ocr" && ocr_available {
            // try OCR text-only
            // Build json for to_translate subset filtered by raw_map
            let mut ocr_json_map = serde_json::Map::new();
            for c in &crops_to_translate {
                if let Some(raw) = raw_map.get(&c.id) { ocr_json_map.insert(c.id.clone(), serde_json::Value::String(raw.clone())); }
            }
            if ocr_json_map.is_empty() {
                // fallback vision if no raw
                for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
                    let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                    let chunk_trans = translate_with_chain(ctx.chain, &mosaic, ctx.prompt, ctx.target_lang, ctx.rate_limiter).await?;
                    if let Some(cache) = ctx.cache { let pn=ctx.chain.primary.name(); let mn=ctx.chain.primary.model_name(); cache.save_batch(&chunk_trans, chunk, ctx.target_lang, pn, mn, ctx.prompt_sig); }
                    all_translations.extend(chunk_trans);
                }
            } else {
                let json_str = serde_json::Value::Object(ocr_json_map).to_string();
                match translate_ocr_json(ctx.chain, ctx.rate_limiter, &json_str, ctx.prompt, ctx.target_lang).await {
                    Ok(map) => {
                        if let Some(cache)=ctx.cache { let pn=ctx.chain.primary.name(); let mn=ctx.chain.primary.model_name(); cache.save_batch(&map, &crops_to_translate, ctx.target_lang, pn, mn, ctx.prompt_sig); }
                        all_translations.extend(map);
                    }
                    Err(e) => {
                        eprintln!("[OCR] text translation failed: {}, fallback vision", e);
                        for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
                            let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                            let chunk_trans = translate_with_chain(ctx.chain, &mosaic, ctx.prompt, ctx.target_lang, ctx.rate_limiter).await?;
                            if let Some(cache) = ctx.cache { let pn=ctx.chain.primary.name(); let mn=ctx.chain.primary.model_name(); cache.save_batch(&chunk_trans, chunk, ctx.target_lang, pn, mn, ctx.prompt_sig); }
                            all_translations.extend(chunk_trans);
                        }
                    }
                }
            }
        } else if ctx.mode == "auto" && ocr_available && !raw_map.is_empty() {
            // auto: try ocr first, fallback vision on failure
            let mut ocr_json_map = serde_json::Map::new();
            for c in &crops_to_translate { if let Some(raw)=raw_map.get(&c.id) { ocr_json_map.insert(c.id.clone(), serde_json::Value::String(raw.clone())); } }
            if ocr_json_map.is_empty() {
                for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
                    let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                    let chunk_trans = translate_with_chain(ctx.chain, &mosaic, ctx.prompt, ctx.target_lang, ctx.rate_limiter).await?;
                    if let Some(cache)=ctx.cache { let pn=ctx.chain.primary.name(); let mn=ctx.chain.primary.model_name(); cache.save_batch(&chunk_trans, chunk, ctx.target_lang, pn, mn, ctx.prompt_sig); }
                    all_translations.extend(chunk_trans);
                }
            } else {
                let json_str = serde_json::Value::Object(ocr_json_map).to_string();
                match translate_ocr_json(ctx.chain, ctx.rate_limiter, &json_str, ctx.prompt, ctx.target_lang).await {
                    Ok(map) => {
                        if let Some(cache)=ctx.cache { let pn=ctx.chain.primary.name(); let mn=ctx.chain.primary.model_name(); cache.save_batch(&map, &crops_to_translate, ctx.target_lang, pn, mn, ctx.prompt_sig); }
                        all_translations.extend(map);
                        // fill missing ids via vision
                        let missing: Vec<CropItem> = crops_to_translate.iter().filter(|c| !all_translations.contains_key(&c.id)).cloned().collect();
                        if !missing.is_empty() {
                            for chunk in missing.chunks(ctx.batch_size.max(1)) {
                                let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                                let chunk_trans = translate_with_chain(ctx.chain, &mosaic, ctx.prompt, ctx.target_lang, ctx.rate_limiter).await?;
                                all_translations.extend(chunk_trans);
                            }
                        }
                    }
                    Err(_) => {
                        for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
                            let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                            let chunk_trans = translate_with_chain(ctx.chain, &mosaic, ctx.prompt, ctx.target_lang, ctx.rate_limiter).await?;
                            if let Some(cache)=ctx.cache { let pn=ctx.chain.primary.name(); let mn=ctx.chain.primary.model_name(); cache.save_batch(&chunk_trans, chunk, ctx.target_lang, pn, mn, ctx.prompt_sig); }
                            all_translations.extend(chunk_trans);
                        }
                    }
                }
            }
        } else {
            // vision
            for chunk in crops_to_translate.chunks(ctx.batch_size.max(1)) {
                let mosaic = MosaicBuilder::build_mosaic(chunk, ctx.font_bytes)?;
                let chunk_trans = translate_with_chain(ctx.chain, &mosaic, ctx.prompt, ctx.target_lang, ctx.rate_limiter).await?;
                if let Some(cache)=ctx.cache { let pn=ctx.chain.primary.name(); let mn=ctx.chain.primary.model_name(); cache.save_batch(&chunk_trans, chunk, ctx.target_lang, pn, mn, ctx.prompt_sig); }
                all_translations.extend(chunk_trans);
            }
        }
    }

    // normalize ft keys to lowercase
    all_translations = norm_map(all_translations);
    for (k,v) in &all_translations { println!("      #{} -> \"{}\"", k, v); }

    // Inpaint + typeset for combined (bubbles + ft)
    let mut page = orig_rgb.clone();
    let inpaint_targets: Vec<kzktdk::model::yolo::Detection> = combined_dets.iter().enumerate().filter(|(i,_)| {
        let id = if *i < detections.len() { (i+1).to_string() } else { format!("ft{}", i - detections.len() +1) };
        if let Some(t)=all_translations.get(&id) { t.to_uppercase()!="SKIP" && !t.trim().is_empty() } else { false }
    }).map(|(_,d)| d.clone()).collect();
    if !inpaint_targets.is_empty() { inpaint_image(&mut page, &inpaint_targets)?; }
    let typesetter = Typesetter::new(ctx.font_bytes, ctx.cjk_font_bytes)?;
    for (idx, det) in combined_dets.iter().enumerate() {
        let id = if idx < detections.len() { (idx+1).to_string() } else { format!("ft{}", idx - detections.len() +1) };
        if let Some(text)=all_translations.get(&id) {
            if text.to_uppercase()=="SKIP" || text.trim().is_empty() { continue; }
            typesetter.render_bubble_text_with_style(&mut page, det, text, Some(ctx.target_lang), None, None);
        }
    }
    page.save(output_path)?;

    if ctx.save_metadata {
        let page_name = input_path.file_name().unwrap_or_default().to_string_lossy().to_string();
        let mut bubbles: Vec<metadata::Bubble> = Vec::new();
        for (i, det) in detections.iter().enumerate() {
            let id = (i+1).to_string();
            bubbles.push(metadata::Bubble { id: id.clone(), bbox: [det.x1,det.y1,det.x2,det.y2], conf: det.conf, translated: all_translations.get(&id).cloned().unwrap_or_default(), bg_color: None, style: None, edited: false, raw_text: raw_map.get(&id).cloned() });
        }
        for (i, fb) in ft_boxes.iter().enumerate() {
            let id = format!("ft{}", i+1);
            // sample bg median not critical
            bubbles.push(metadata::Bubble { id: id.clone(), bbox: *fb, conf: 0.90, translated: all_translations.get(&id).cloned().unwrap_or_default(), bg_color: None, style: None, edited: false, raw_text: raw_map.get(&id).cloned() });
        }
        let mut data = metadata::PageEditData::new(page_name.clone(), w_img, h_img, ctx.target_lang.to_string(), ctx.prompt_sig.to_string(), bubbles);
        if ctx.ocr != "none" { data.ocr_engine = Some(ctx.ocr.clone()); }
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
