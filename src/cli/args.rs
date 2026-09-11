use clap::{Parser, Subcommand};
use std::path::PathBuf;

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
  kzktdk metadata preview <image> --metadata page.kedit.json --id 1 --text \"Hi\" --to-stdout --thumb 512
  kzktdk metadata show <json> [--id 1]           Show bubbles table
  kzktdk metadata watch --project project.kedit.json --images orig/ -o rendered/ --once
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
    version = "0.2.0",
    about = "KZKT Desktop - High-Performance Comic & Manga Translation CLI",
    help_template = MAIN_HELP_TEMPLATE
)]
pub struct Cli {
    /// Hidden developer reference manual
    #[arg(long, hide = true)]
    pub dev_help: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
pub enum Commands {
    /// Decrypt the bundled/custom model (.dat -> .onnx)
    #[command(hide = true)]
    DecryptModel {
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_MODEL_DAT_PATH)]
        source: PathBuf,
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_MODEL_PATH)]
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
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_MODEL_PATH)]
        model: PathBuf,
        /// Export detections to JSON file
        #[arg(long)]
        json: Option<PathBuf>,
        /// Jobs for batch parallel: auto or number
        #[arg(long, default_value_t = kzktdk::config::DEFAULT_JOBS.to_string())]
        jobs: String,
        /// Also detect freetext outside bubbles (requires --ocr)
        #[arg(long, default_value_t = false)]
        translate_free_text: bool,
        /// OCR script for freetext: auto|jp|en|kr|cn
        #[arg(long, default_value = "jp")]
        ocr_script: String,
        /// OCR engine for freetext: none|rapid (JP/EN/KR/CN+auto)|tesseract|manga (NOTE: manga deprecated, use rapid instead)
        #[arg(long, default_value = "none", value_parser = clap::builder::PossibleValuesParser::new(["none", "rapid", "manga", "tesseract", "vision", "local"]))]
        ocr: String,
        /// Custom OCR model path override
        #[arg(long)]
        ocr_model: Option<PathBuf>,
        /// Output format: text (previews + logs) or json (detections array to stdout)
        #[arg(long, default_value = "text", value_parser = clap::builder::PossibleValuesParser::new(["text", "json"]))]
        format: String,
        /// Suppress informational logs (data goes to stdout, logs to stderr)
        #[arg(long, default_value_t = false)]
        quiet: bool,
    },

    /// Inpaint/erase original text inside dialogue bubbles
    #[command(hide = true)]
    Inpaint {
        /// Input image path
        input: PathBuf,
        /// Output inpainted image path
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_INPAINT_OUTPUT)]
        output: PathBuf,
        /// ONNX model path
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_MODEL_PATH)]
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
        /// Input path: image (.jpg/.png/.webp), folder, or comic archive (.cbz/.zip/.epub/.pdf)
        input: Option<PathBuf>,
        /// Output path (image file, folder, or .cbz file)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Export format: 'folder', 'cbz', 'pdf', or 'auto' (defaults to 'auto')
        #[arg(long, default_value = "auto")]
        export: String,
        /// ONNX model path
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_MODEL_PATH)]
        model: PathBuf,
        /// Target translation language
        #[arg(short, long, default_value_t = kzktdk::config::DEFAULT_TARGET_LANG.to_string())]
        target_lang: String,
        /// Custom prompt instructions or additional translation rules
        #[arg(long)]
        prompt: Option<String>,
        /// Number of dialogue bubbles to batch per LLM translation request
        #[arg(long, default_value_t = kzktdk::config::DEFAULT_CLI_BATCH_SIZE)]
        batch_size: usize,
        /// LLM Provider: gemini, openai, ollama, or claude
        #[arg(short, long, default_value_t = kzktdk::config::DEFAULT_PROVIDER.to_string())]
        provider: String,
        /// Fallback providers comma-separated (e.g. openai,claude)
        #[arg(long)]
        fallback_provider: Option<String>,
        /// Rate limit RPS
        #[arg(long, default_value_t = kzktdk::config::DEFAULT_RATE_LIMIT_RPS)]
        rate_limit: u32,
        /// Jobs for batch parallel: auto or number (default auto = num_cpus)
        #[arg(long, default_value_t = kzktdk::config::DEFAULT_JOBS.to_string())]
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
        #[arg(long, default_value_t = kzktdk::config::OPENAI_DEFAULT_BASE_URL.to_string())]
        openai_base_url: String,
        /// Model name for OpenAI / Ollama
        #[arg(long, default_value_t = kzktdk::config::DEFAULT_OPENAI_MODEL.to_string())]
        openai_model: String,
        /// Model name for Gemini
        #[arg(long, default_value_t = kzktdk::config::DEFAULT_GEMINI_MODEL.to_string())]
        gemini_model: String,
        /// Claude API Key (or set ANTHROPIC_API_KEY env var)
        #[arg(long, env = "ANTHROPIC_API_KEY")]
        claude_key: Option<String>,
        /// Model name for Claude
        #[arg(long, default_value_t = kzktdk::config::DEFAULT_CLAUDE_MODEL.to_string())]
        claude_model: String,
        /// Primary comic font (default: Komika Axis like KZKT mobile)
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_FONT_PATH)]
        font: PathBuf,
        /// Secondary / CJK font for non-Latin text (default: KosugiMaru)
        #[arg(long, default_value = kzktdk::config::DEFAULT_CJK_FONT_PATH)]
        cjk_font: PathBuf,
        /// Save metadata sidecar .kedit.json per page + project.kedit.json
        #[arg(long)]
        save_metadata: bool,
        /// Custom metadata directory (default: same as output)
        #[arg(long)]
        metadata_dir: Option<PathBuf>,
        /// OCR engine: none (default), rapid (JP/EN/KR/CN+auto), tesseract, manga (deprecated) — pluggable (tesseract needs its binary)
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
        /// Glossary file: JSON object mapping source term -> required translation
        #[arg(long)]
        glossary: Option<PathBuf>,
        /// Progress output: text or jsonl (JSON lines to stderr)
        #[arg(long, default_value = "text", value_parser = clap::builder::PossibleValuesParser::new(["text", "jsonl"]))]
        progress: String,
        /// Machine-readable final summary format: text or json (JSON to stdout)
        #[arg(long, default_value = "text", value_parser = clap::builder::PossibleValuesParser::new(["text", "json"]))]
        format: String,
        /// Suppress informational logs (data goes to stdout, logs to stderr)
        #[arg(long, default_value_t = false)]
        quiet: bool,
        /// Retry only failed pages from a previous output folder (skip pages with good output)
        #[arg(long)]
        retry_failed: Option<PathBuf>,
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
pub enum MetadataCmd {
    /// Export detections to JSON without LLM (cheap)
    Export {
        /// Input path: image, folder, or archive
        input: PathBuf,
        /// Output JSON file
        #[arg(short, long)]
        json: PathBuf,
        /// ONNX model path
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_MODEL_PATH)]
        model: PathBuf,
        /// Initial reading order from bubble geometry: r2l (manga), l2r (western), off
        #[arg(long, default_value = "r2l", value_parser = clap::builder::PossibleValuesParser::new(["r2l", "l2r", "off"]))]
        reading_order: String,
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
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_FONT_PATH)]
        font: PathBuf,
        /// CJK font path
        #[arg(long, default_value = kzktdk::config::DEFAULT_CJK_FONT_PATH)]
        cjk_font: PathBuf,
        /// Jobs for batch render (auto or N)
        #[arg(long, default_value_t = kzktdk::config::DEFAULT_JOBS.to_string())]
        jobs: String,
        /// Progress output: text or jsonl (JSON lines to stderr)
        #[arg(long, default_value = "text", value_parser = clap::builder::PossibleValuesParser::new(["text", "jsonl"]))]
        progress: String,
        /// Suppress informational logs (data goes to stdout, logs to stderr)
        #[arg(long, default_value_t = false)]
        quiet: bool,
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
        /// Output preview image (required unless --to-stdout)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Print PNG as single-line base64 to stdout instead of writing a file
        #[arg(long, default_value_t = false)]
        to_stdout: bool,
        /// Downscale longest side to N px before encoding (0 = no resize)
        #[arg(long, default_value_t = 0)]
        thumb: u32,
        /// Font path
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_FONT_PATH)]
        font: PathBuf,
        /// CJK font path
        #[arg(long, default_value = kzktdk::config::DEFAULT_CJK_FONT_PATH)]
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
        /// Suppress informational logs (data goes to stdout, logs to stderr)
        #[arg(long, default_value_t = false)]
        quiet: bool,
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
        /// Transactional JSON patch from stdin (EditPatch); aborts without writing on any error
        #[arg(long, default_value_t = false)]
        stdin_patch: bool,
        /// Set reading order: "1,ft1,2" (comma-separated bubble IDs)
        #[arg(long, value_name = "ORDER")]
        order: Option<String>,
        /// Set mask override per bubble: "ID=/path/to/mask.png" (empty to clear)
        #[arg(long, value_name = "ID=Path")]
        mask_path: Vec<String>,
    },
    /// Validate metadata JSON
    Validate {
        /// JSON file to validate
        json: PathBuf,
        /// Output format: text or json
        #[arg(long, default_value = "text", value_parser = clap::builder::PossibleValuesParser::new(["text", "json"]))]
        format: String,
        /// Suppress informational logs (data goes to stdout, logs to stderr)
        #[arg(long, default_value_t = false)]
        quiet: bool,
    },
    /// Show metadata as table (auto-detect PageEditData or Project)
    Show {
        /// JSON file (PageEditData .kedit.json or project.kedit.json)
        json: PathBuf,
        /// Filter by bubble ID (only for PageEditData)
        #[arg(long)]
        id: Option<String>,
        /// Output format: text (table) or json (raw metadata)
        #[arg(long, default_value = "text", value_parser = clap::builder::PossibleValuesParser::new(["text", "json"]))]
        format: String,
        /// Suppress informational logs (data goes to stdout, logs to stderr)
        #[arg(long, default_value_t = false)]
        quiet: bool,
    },
    /// Pack rendered folder into CBZ archive
    Pack {
        /// Input folder with rendered images
        input: PathBuf,
        /// Output CBZ file
        #[arg(short, long)]
        output: PathBuf,
        /// Output format: text or json (summary to stdout)
        #[arg(long, default_value = "text", value_parser = clap::builder::PossibleValuesParser::new(["text", "json"]))]
        format: String,
    },
    /// Set or clear a custom brush mask override for a bubble
    Mask {
        /// Metadata JSON file
        json: PathBuf,
        /// Bubble ID
        #[arg(long)]
        id: String,
        /// Mask image path (grayscale; white = inpaint). Omit with --clear.
        #[arg(long)]
        from: Option<PathBuf>,
        /// Clear the mask override
        #[arg(long, default_value_t = false)]
        clear: bool,
    },
    /// Watch project for changes and incrementally re-render dirty pages
    Watch {
        /// Project JSON file
        #[arg(long)]
        project: PathBuf,
        /// Images folder (overrides project image paths)
        #[arg(long)]
        images: Option<PathBuf>,
        /// Output folder for rendered pages
        #[arg(short, long)]
        output: PathBuf,
        /// Font path
        #[arg(short, long, default_value = kzktdk::config::DEFAULT_FONT_PATH)]
        font: PathBuf,
        /// CJK font path
        #[arg(long, default_value = kzktdk::config::DEFAULT_CJK_FONT_PATH)]
        cjk_font: PathBuf,
        /// Poll interval in seconds
        #[arg(long, default_value_t = kzktdk::config::DEFAULT_WATCH_INTERVAL_SECS)]
        interval: u64,
        /// Render once and exit (for scripts/tests)
        #[arg(long, default_value_t = false)]
        once: bool,
    },
}

#[derive(Subcommand)]
pub enum FontCmd {
    /// Import font file into registry
    Import {
        /// Path to TTF/OTF file
        path: PathBuf,
        /// Custom name (default: file stem)
        #[arg(long)]
        name: Option<String>,
    },
    /// List imported fonts
    List {
        /// Output format: text or json
        #[arg(long, default_value = "text", value_parser = clap::builder::PossibleValuesParser::new(["text", "json"]))]
        format: String,
        /// Suppress informational logs
        #[arg(long, default_value_t = false)]
        quiet: bool,
    },
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
    GetDefault {
        /// Output format: text or json
        #[arg(long, default_value = "text", value_parser = clap::builder::PossibleValuesParser::new(["text", "json"]))]
        format: String,
        /// Suppress informational logs
        #[arg(long, default_value_t = false)]
        quiet: bool,
    },
}
