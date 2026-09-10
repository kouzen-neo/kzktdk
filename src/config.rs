//! Central tuning + endpoint constants.
//!
//! All previously-scattered magic literals (LLM endpoints, CLI defaults,
//! retry/backoff tuning, vision thresholds) live here with a tuning note
//! each. CLI `default_value_t` attributes reference these constants so the
//! `--help` output stays byte-identical.
//!
//! NOTE: `tauri::TranslateConfig::default()` intentionally keeps its own GUI
//! defaults (e.g. a different default Gemini model); only values identical
//! to the CLI are shared from here.

// ---------------------------------------------------------------------------
// LLM endpoints
// ---------------------------------------------------------------------------

/// Default OpenAI-compatible base URL (also used for Ollama overrides).
pub const OPENAI_DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Google Generative Language API base.
pub const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com";

/// Anthropic Messages API endpoint (full URL, no per-model path).
pub const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";

/// Anthropic API version header required alongside the key.
pub const ANTHROPIC_API_VERSION: &str = "2023-06-01";

/// Build the Gemini `generateContent` URL (key passed as query param).
pub fn gemini_generate_url(model: &str, api_key: &str) -> String {
    format!("{GEMINI_API_BASE}/v1beta/models/{model}:generateContent?key={api_key}")
}

// ---------------------------------------------------------------------------
// CLI defaults (single source; referenced via `default_value_t`)
// ---------------------------------------------------------------------------

/// Default translation target language.
pub const DEFAULT_TARGET_LANG: &str = "English";

/// Default LLM provider key.
pub const DEFAULT_PROVIDER: &str = "gemini";

/// Default request rate (requests per second) for the provider limiter.
pub const DEFAULT_RATE_LIMIT_RPS: u32 = 3;

/// Default max retry attempts per LLM call (see backoff consts below).
pub const RATE_LIMIT_RETRY_MAX: u32 = 3;

/// Default bubbles-per-LLM-request for the CLI batch pipeline.
/// (GUI default differs; see `tauri::TranslateConfig`.)
pub const DEFAULT_CLI_BATCH_SIZE: usize = 15;

/// Default Gemini model for the CLI.
/// (GUI default differs; see `tauri::TranslateConfig`.)
pub const DEFAULT_GEMINI_MODEL: &str = "gemini-3.1-flash-lite";

/// Default OpenAI model for the CLI.
pub const DEFAULT_OPENAI_MODEL: &str = "gpt-4o-mini";

/// Default Claude model for the CLI.
/// (GUI default differs; see `tauri::TranslateConfig`.)
pub const DEFAULT_CLAUDE_MODEL: &str = "claude-3-5-sonnet-20241022";

/// Default bundled ONNX bubble-detection model.
pub const DEFAULT_MODEL_PATH: &str = "models/kzkt.onnx";

/// Encrypted model source auto-decrypted when the ONNX file is missing.
pub const DEFAULT_MODEL_DAT_PATH: &str = "models/kzkt.dat";

/// Default Latin comic font.
pub const DEFAULT_FONT_PATH: &str = "fonts/Komika Axis.ttf";

/// Default CJK fallback font.
pub const DEFAULT_CJK_FONT_PATH: &str = "fonts/KosugiMaru.ttf";

/// Default `inpaint` output file.
pub const DEFAULT_INPAINT_OUTPUT: &str = "inpainted.png";

/// Default `--jobs` selector (resolve via `available_parallelism`).
pub const DEFAULT_JOBS: &str = "auto";

/// Fallback worker count when CPU parallelism cannot be detected.
pub const DEFAULT_JOBS_FALLBACK: usize = 4;

/// Default `metadata watch` poll interval in seconds.
pub const DEFAULT_WATCH_INTERVAL_SECS: u64 = 1;

// ---------------------------------------------------------------------------
// LLM request tuning
// ---------------------------------------------------------------------------

/// Sampling temperature for translation calls (low = faithful, literal).
pub const LLM_TEMPERATURE: f32 = 0.2;

/// Max output tokens for Claude translation calls.
pub const CLAUDE_MAX_TOKENS: u32 = 1024;

// ---------------------------------------------------------------------------
// Retry / backoff tuning (`RateLimiter::execute_with_retry`)
// ---------------------------------------------------------------------------

/// Base backoff step: actual delay is `BASE * 2^min(attempt-1, MAX_SHIFT)`.
pub const RETRY_BACKOFF_BASE_MS: u64 = 1000;

/// Caps the exponential growth (`BASE * 2^3 = 8s` max per wait).
pub const RETRY_BACKOFF_MAX_SHIFT: u32 = 3;

// ---------------------------------------------------------------------------
// Vision / geometry tuning
// ---------------------------------------------------------------------------

/// Divisor mapping u8 channels to unit floats for the YOLO tensor.
/// (Divisor form keeps bit-identical results vs the old `/ 255.0`.)
pub const IMAGE_U8_DIVISOR: f32 = 255.0;

/// Gap (px) below which nearby OCR boxes are merged into one text box.
pub const MERGE_NEARBY_GAP_PX: u32 = 20;

// ---------------------------------------------------------------------------
// Metadata lock-file tuning
// ---------------------------------------------------------------------------

/// How many times to spin waiting for a stale `.lock` file.
pub const METADATA_LOCK_SPIN_ATTEMPTS: usize = 50;

/// Sleep between lock-file polls.
pub const METADATA_LOCK_POLL_MS: u64 = 10;

// ---------------------------------------------------------------------------
// OCR model download URLs (auto-fetch on first `--ocr rapid` use)
// ---------------------------------------------------------------------------

/// RapidOCR detection model (PP-OCRv3).
pub const RAPIDOCR_DET_URL: &str =
    "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv3/ch_PP-OCRv3_det_infer.onnx";

/// RapidOCR Japanese recognition model (PP-OCRv1 CRNN).
pub const RAPIDOCR_REC_URL: &str =
    "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv1/japan_rec_crnn.onnx";

/// PaddleOCR Japanese character dictionary.
pub const PADDLE_JAPAN_DICT_URL: &str =
    "https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/main/ppocr/utils/dict/japan_dict.txt";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gemini_url_matches_legacy_format() {
        // Guards the Fase-2 refactor: byte-identical to the old inline format!.
        assert_eq!(
            gemini_generate_url("gemini-3.1-flash-lite", "KEY"),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.1-flash-lite:generateContent?key=KEY"
        );
    }

    #[test]
    fn retry_backoff_schedule_unchanged() {
        // Old: 1000 * (1 << (attempt-1).min(3)) ms for attempts 1..=4.
        let expected = [1000u64, 2000, 4000, 8000];
        for (i, want) in expected.iter().enumerate() {
            let attempt = (i + 1) as u32;
            let got = RETRY_BACKOFF_BASE_MS * (1u64 << (attempt - 1).min(RETRY_BACKOFF_MAX_SHIFT));
            assert_eq!(got, *want, "attempt {}", attempt);
        }
    }
}
