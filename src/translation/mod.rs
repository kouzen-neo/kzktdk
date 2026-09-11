use anyhow::{Context, Result, bail};
use image::{DynamicImage, Rgb, RgbImage};
use reqwest::header::{HeaderMap, HeaderValue};
use std::collections::HashMap;
use std::io::Cursor;
use std::time::Duration;

use crate::config;
use crate::model::yolo::Detection;
use crate::typesetting::Typesetter;

pub struct CropItem {
    pub id: String,
    pub image: RgbImage,
}

impl Clone for CropItem {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),
            image: self.image.clone(),
        }
    }
}

pub struct MosaicBuilder;

impl MosaicBuilder {
    /// Builds a mosaic image containing crops arranged vertically with red numbered labels on the left.
    pub fn build_mosaic(crops: &[CropItem], font_data: &[u8]) -> Result<DynamicImage> {
        if crops.is_empty() {
            return Ok(DynamicImage::ImageRgb8(RgbImage::from_pixel(
                100,
                100,
                Rgb([255, 255, 255]),
            )));
        }

        let max_crop_w = crops.iter().map(|c| c.image.width()).max().unwrap_or(100);
        let total_crop_h: u32 = crops.iter().map(|c| c.image.height()).sum();

        let margin_left = 70u32;
        let margin_right = 20u32;
        let spacing = 15u32;

        let mosaic_w = (max_crop_w + margin_left + margin_right).max(400);
        let mosaic_h = total_crop_h + (crops.len() as u32 * spacing) + 30;

        let mut mosaic = RgbImage::from_pixel(mosaic_w, mosaic_h, Rgb([255, 255, 255]));
        let typesetter = Typesetter::new(font_data, None)?;

        let mut curr_y = 15u32;
        for crop in crops {
            let crop_h = crop.image.height();
            let crop_w = crop.image.width();

            // Draw red ID text on the left
            let id_box = Detection {
                x1: 5,
                y1: curr_y + (crop_h.saturating_sub(40)) / 2,
                x2: margin_left - 10,
                y2: curr_y + (crop_h.saturating_sub(40)) / 2 + 40,
                conf: 1.0,
            };
            typesetter.render_bubble_text(
                &mut mosaic,
                &id_box,
                &crop.id,
                None,
                Some(Rgb([220, 20, 20])), // Red ID
            );

            // Blit crop image
            for cy in 0..crop_h {
                for cx in 0..crop_w {
                    mosaic.put_pixel(margin_left + cx, curr_y + cy, *crop.image.get_pixel(cx, cy));
                }
            }

            curr_y += crop_h + spacing;
        }

        Ok(DynamicImage::ImageRgb8(mosaic))
    }
}

pub fn build_translation_prompt(target_lang: &str) -> String {
    build_translation_prompt_with_glossary(target_lang, None)
}

/// Glossary-aware prompt builder. `glossary` maps source term -> required
/// target term (e.g. character names). Injected before custom rules so the
/// caller can still append `ADDITIONAL RULES` afterwards.
pub fn build_translation_prompt_with_glossary(
    target_lang: &str,
    glossary: Option<&std::collections::BTreeMap<String, String>>,
) -> String {
    let id_extra = if target_lang.eq_ignore_ascii_case("indonesian")
        || target_lang.eq_ignore_ascii_case("bahasa indonesia")
    {
        "\nINDONESIAN RULES:\n- Use natural conversational manga dialogue (e.g. use natural flow like aku/kamu, natural sentence flow, avoid stiff textbook phrasing unless formal).\n- Context: 'full' / 'いっぱい' / '満員' referring to places, stores, rooms, or queues means 'penuh' or 'ramai', NOT 'kenyang'; only use 'kenyang' when explicitly discussing stomach fullness after eating.\n"
    } else {
        ""
    };

    let example = if target_lang.eq_ignore_ascii_case("indonesian")
        || target_lang.eq_ignore_ascii_case("bahasa indonesia")
    {
        r#"{"1": "Halo!", "2": "SKIP", "3": "Ayo pergi!"}"#
    } else {
        r#"{"1": "Hello!", "2": "SKIP", "3": "Let's go!"}"#
    };

    let glossary_block = match glossary {
        Some(g) if !g.is_empty() => {
            let mut lines = String::from(
                "\nGLOSSARY (must obey): Always translate the following terms exactly as given, wherever they appear:\n",
            );
            for (src, dst) in g.iter() {
                lines.push_str(&format!("- \"{}\" -> \"{}\"\n", src, dst));
            }
            lines
        }
        _ => String::new(),
    };

    format!(
        r#"You are an accurate, natural manga and comic translator translating into {target_lang}.
The image contains speech bubbles and freetext (captions/SFX) arranged vertically.
Each item is prefixed with a LARGE RED ID on its left (numeric like "1" for bubbles, or "ft1"/"ft2" for freetext outside bubbles).

MAIN TASK:
Read the dialogue in each item and translate it faithfully into {target_lang}.
{glossary_block}
RULES:
1. Natural flow: Use natural conversational manga dialogue suitable for speech bubbles.{id_extra}
2. Honorifics: Keep Japanese honorifics (san, kun, chan, sama, senpai, etc.) as-is.
3. If an item contains only sound effects (SFX) or unreadable art, reply with "SKIP" — but still include the key (e.g. "ft1":"SKIP").
4. Return a valid JSON object mapping every visible red ID (including "ft1","ft2") to its translation. Include ALL IDs even if SKIP.
5. Example: {example}  (if freetext present, also include "ft1":"Hello outside")

Output ONLY the raw JSON object, without markdown fences."#
    )
}

/// Parses the JSON mapping from LLM response, stripping any markdown backticks.
pub fn parse_translation_json(raw: &str) -> Result<HashMap<String, String>> {
    let cleaned = raw
        .trim()
        .strip_prefix("```json")
        .or_else(|| raw.trim().strip_prefix("```"))
        .unwrap_or(raw.trim());
    let cleaned = cleaned.strip_suffix("```").unwrap_or(cleaned).trim();

    // Find first { and last }
    let start = cleaned
        .find('{')
        .context("No JSON object found in response")?;
    let end = cleaned.rfind('}').context("No closing brace in response")?;
    let json_slice = &cleaned[start..=end];

    let map: HashMap<String, String> = serde_json::from_str(json_slice)
        .with_context(|| format!("Failed to parse translation JSON: {}", json_slice))?;
    Ok(map)
}

pub enum Provider {
    Gemini {
        api_key: String,
        model: String,
    },
    OpenAI {
        api_key: String,
        base_url: String,
        model: String,
    },
    Claude {
        api_key: String,
        model: String,
    },
}

static HTTP_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

pub fn http_client() -> &'static reqwest::Client {
    HTTP_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .connect_timeout(Duration::from_secs(20))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

impl Provider {
    pub fn name(&self) -> &str {
        match self {
            Provider::Gemini { .. } => "gemini",
            Provider::OpenAI { .. } => "openai",
            Provider::Claude { .. } => "claude",
        }
    }

    pub fn model_name(&self) -> &str {
        match self {
            Provider::Gemini { model, .. } => model,
            Provider::OpenAI { model, .. } => model,
            Provider::Claude { model, .. } => model,
        }
    }

    /// Raw text response from provider (before JSON parsing), for repair/fallback handling
    pub async fn translate_mosaic_raw(
        &self,
        mosaic: &DynamicImage,
        prompt: &str,
    ) -> Result<String> {
        let mut jpeg_bytes = Vec::new();
        mosaic.write_to(&mut Cursor::new(&mut jpeg_bytes), image::ImageFormat::Jpeg)?;
        let base64_image =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg_bytes);

        let client = http_client();

        let response_text = match self {
            Provider::Gemini { api_key, model } => {
                let url = config::gemini_generate_url(model, api_key);

                let body = serde_json::json!({
                    "contents": [{
                        "parts": [
                            { "text": prompt },
                            {
                                "inline_data": {
                                    "mime_type": "image/jpeg",
                                    "data": base64_image
                                }
                            }
                        ]
                    }],
                    "generationConfig": {
                        "temperature": config::LLM_TEMPERATURE,
                        "response_mime_type": "application/json"
                    }
                });

                let res = client.post(&url).json(&body).send().await?;
                if !res.status().is_success() {
                    bail!(
                        "Gemini API error: HTTP {} - {}",
                        res.status(),
                        res.text().await?
                    );
                }

                let json: serde_json::Value = res.json().await?;
                json["candidates"][0]["content"]["parts"][0]["text"]
                    .as_str()
                    .context("No text in Gemini response")?
                    .to_string()
            }
            Provider::OpenAI {
                api_key,
                base_url,
                model,
            } => {
                let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

                let body = serde_json::json!({
                    "model": model,
                    "messages": [{
                        "role": "user",
                        "content": [
                            { "type": "text", "text": prompt },
                            {
                                "type": "image_url",
                                "image_url": {
                                    "url": format!("data:image/jpeg;base64,{}", base64_image)
                                }
                            }
                        ]
                    }],
                    "temperature": config::LLM_TEMPERATURE,
                    "response_format": { "type": "json_object" }
                });

                let mut headers = HeaderMap::new();
                if !api_key.is_empty() {
                    headers.insert(
                        "Authorization",
                        HeaderValue::from_str(&format!("Bearer {}", api_key))?,
                    );
                }

                let res = client
                    .post(&url)
                    .headers(headers)
                    .json(&body)
                    .send()
                    .await?;
                if !res.status().is_success() {
                    bail!(
                        "OpenAI API error: HTTP {} - {}",
                        res.status(),
                        res.text().await?
                    );
                }

                let json: serde_json::Value = res.json().await?;
                json["choices"][0]["message"]["content"]
                    .as_str()
                    .context("No content in OpenAI response")?
                    .to_string()
            }
            Provider::Claude { api_key, model } => {
                let url = config::ANTHROPIC_API_URL;

                let body = serde_json::json!({
                    "model": model,
                    "max_tokens": config::CLAUDE_MAX_TOKENS,
                    "messages": [{
                        "role": "user",
                        "content": [
                            {
                                "type": "image",
                                "source": {
                                    "type": "base64",
                                    "media_type": "image/jpeg",
                                    "data": base64_image
                                }
                            },
                            { "type": "text", "text": prompt }
                        ]
                    }]
                });

                let mut headers = HeaderMap::new();
                headers.insert("x-api-key", HeaderValue::from_str(api_key)?);
                headers.insert(
                    "anthropic-version",
                    HeaderValue::from_static(config::ANTHROPIC_API_VERSION),
                );

                let res = client.post(url).headers(headers).json(&body).send().await?;
                if !res.status().is_success() {
                    bail!(
                        "Claude API error: HTTP {} - {}",
                        res.status(),
                        res.text().await?
                    );
                }

                let json: serde_json::Value = res.json().await?;
                json["content"][0]["text"]
                    .as_str()
                    .context("No text in Claude response")?
                    .to_string()
            }
        };
        Ok(response_text)
    }

    pub async fn translate_mosaic(
        &self,
        mosaic: &DynamicImage,
        prompt: &str,
    ) -> Result<HashMap<String, String>> {
        let raw = self.translate_mosaic_raw(mosaic, prompt).await?;
        parse_translation_json(&raw)
    }

    /// Text-only translation for JSON repair
    pub async fn translate_text(&self, text: &str, prompt: &str) -> Result<String> {
        let client = http_client();
        let full_prompt = format!("{}\n\nInput:\n{}", prompt, text);
        match self {
            Provider::Gemini { api_key, model } => {
                let url = config::gemini_generate_url(model, api_key);
                let body = serde_json::json!({
                    "contents": [{ "parts": [{ "text": full_prompt }] }],
                    "generationConfig": { "temperature": config::LLM_TEMPERATURE, "response_mime_type": "application/json" }
                });
                let res = client.post(&url).json(&body).send().await?;
                if !res.status().is_success() {
                    bail!(
                        "Gemini repair error: HTTP {} - {}",
                        res.status(),
                        res.text().await?
                    );
                }
                let json: serde_json::Value = res.json().await?;
                Ok(json["candidates"][0]["content"]["parts"][0]["text"]
                    .as_str()
                    .context("No text in Gemini repair response")?
                    .to_string())
            }
            Provider::OpenAI {
                api_key,
                base_url,
                model,
            } => {
                let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
                let body = serde_json::json!({
                    "model": model,
                    "messages": [{ "role": "user", "content": full_prompt }],
                    "temperature": config::LLM_TEMPERATURE,
                    "response_format": { "type": "json_object" }
                });
                let mut headers = HeaderMap::new();
                if !api_key.is_empty() {
                    headers.insert(
                        "Authorization",
                        HeaderValue::from_str(&format!("Bearer {}", api_key))?,
                    );
                }
                let res = client
                    .post(&url)
                    .headers(headers)
                    .json(&body)
                    .send()
                    .await?;
                if !res.status().is_success() {
                    bail!(
                        "OpenAI repair error: HTTP {} - {}",
                        res.status(),
                        res.text().await?
                    );
                }
                let json: serde_json::Value = res.json().await?;
                Ok(json["choices"][0]["message"]["content"]
                    .as_str()
                    .context("No content in OpenAI repair")?
                    .to_string())
            }
            Provider::Claude { api_key, model } => {
                let url = config::ANTHROPIC_API_URL;
                let body = serde_json::json!({
                    "model": model,
                    "max_tokens": config::CLAUDE_MAX_TOKENS,
                    "messages": [{ "role": "user", "content": full_prompt }]
                });
                let mut headers = HeaderMap::new();
                headers.insert("x-api-key", HeaderValue::from_str(api_key)?);
                headers.insert(
                    "anthropic-version",
                    HeaderValue::from_static(config::ANTHROPIC_API_VERSION),
                );
                let res = client.post(url).headers(headers).json(&body).send().await?;
                if !res.status().is_success() {
                    bail!(
                        "Claude repair error: HTTP {} - {}",
                        res.status(),
                        res.text().await?
                    );
                }
                let json: serde_json::Value = res.json().await?;
                Ok(json["content"][0]["text"]
                    .as_str()
                    .context("No text in Claude repair")?
                    .to_string())
            }
        }
    }
}

// --- Provider Chain + Rate Limiter + Repair ---

pub struct ProviderChain {
    pub primary: Provider,
    pub fallbacks: Vec<Provider>,
}

impl ProviderChain {
    pub fn all_providers(&self) -> Vec<&Provider> {
        let mut v = vec![&self.primary];
        v.extend(self.fallbacks.iter());
        v
    }
}

pub struct RateLimiter {
    pub max_rps: u32,
    pub retry_max: u32,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self {
            max_rps: config::DEFAULT_RATE_LIMIT_RPS,
            retry_max: config::RATE_LIMIT_RETRY_MAX,
        }
    }
}

impl RateLimiter {
    pub fn new(max_rps: u32) -> Self {
        Self {
            max_rps: max_rps.max(1),
            retry_max: config::RATE_LIMIT_RETRY_MAX,
        }
    }

    pub async fn execute_with_retry<F, Fut>(&self, verbose: bool, mut api_call: F) -> Result<String>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<String>>,
    {
        let mut last_err: Option<anyhow::Error> = None;
        for attempt in 0..=self.retry_max {
            if attempt > 0 {
                let backoff = Duration::from_millis(
                    config::RETRY_BACKOFF_BASE_MS
                        * (1u64 << (attempt - 1).min(config::RETRY_BACKOFF_MAX_SHIFT)),
                );
                if verbose {
                    println!(
                        "  [RateLimit] Retry {}/{} after {}ms",
                        attempt,
                        self.retry_max,
                        backoff.as_millis()
                    );
                } else {
                    eprintln!(
                        "  [RateLimit] Retry {}/{} after {}ms",
                        attempt,
                        self.retry_max,
                        backoff.as_millis()
                    );
                }
                tokio::time::sleep(backoff).await;
            }
            // simple token bucket delay
            if self.max_rps > 0 && attempt == 0 {
                // minimal pacing: 1/max_rps interval handled by sleep between calls externally if needed
            }
            match api_call().await {
                Ok(s) => return Ok(s),
                Err(e) => {
                    let msg = e.to_string();
                    let is_retryable = msg.contains("429")
                        || msg.contains("503")
                        || msg.contains("502")
                        || msg.contains("500")
                        || msg.contains("rate");
                    if is_retryable && attempt < self.retry_max {
                        last_err = Some(e);
                        continue;
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("RateLimiter exhausted retries")))
    }
}

pub async fn translate_with_chain(
    chain: &ProviderChain,
    mosaic: &DynamicImage,
    prompt: &str,
    target_lang: &str,
    rate_limiter: &RateLimiter,
    verbose: bool,
) -> Result<HashMap<String, String>> {
    macro_rules! cinfo {
        ($($t:tt)*) => {
            if verbose { println!($($t)*) } else { eprintln!($($t)*) }
        };
    }
    for prov in chain.all_providers() {
        cinfo!(
            "  Translating with {} ({})...",
            prov.name(),
            prov.model_name()
        );
        let raw_result = rate_limiter
            .execute_with_retry(verbose, || prov.translate_mosaic_raw(mosaic, prompt))
            .await;
        match raw_result {
            Ok(raw) => {
                match parse_translation_json(&raw) {
                    Ok(map) if !map.is_empty() => return Ok(map),
                    _ => {
                        // Try repair
                        cinfo!(
                            "  [!] {} returned unparseable output (raw: {}). Trying repair...",
                            prov.name(),
                            raw.chars().take(80).collect::<String>()
                        );
                        if let Some(repaired) =
                            repair_json_output(prov, &raw, target_lang, rate_limiter, verbose).await
                            && let Ok(map) = parse_translation_json(&repaired)
                            && !map.is_empty()
                        {
                            cinfo!("  [Repair] {} repair succeeded", prov.name());
                            return Ok(map);
                        }
                        cinfo!(
                            "  [Failover] {} failed (unparseable). Trying next provider...",
                            prov.name()
                        );
                        continue;
                    }
                }
            }
            Err(e) => {
                cinfo!(
                    "  [Failover] {} failed ({}). Trying fallback...",
                    prov.name(),
                    e
                );
                continue;
            }
        }
    }
    bail!("All providers failed to produce a valid translation")
}

async fn repair_json_output(
    prov: &Provider,
    raw: &str,
    target_lang: &str,
    rate_limiter: &RateLimiter,
    verbose: bool,
) -> Option<String> {
    if raw.trim().is_empty() {
        return None;
    }
    let repair_prompt = format!(
        "The text below is a translation result in {target_lang} that is not valid JSON. Fix ONLY the JSON syntax/formatting errors and return the exact same translations in {target_lang} as a valid JSON object with the same keys and values. KEEP all translations in {target_lang} — do NOT translate back to English. Output ONLY the corrected JSON object — no markdown, no commentary.\n\nBroken output:\n{raw}"
    );
    if verbose {
        println!("  [!] Requesting JSON repair from {}...", prov.name());
    } else {
        eprintln!("  [!] Requesting JSON repair from {}...", prov.name());
    }
    let res = rate_limiter
        .execute_with_retry(verbose, || prov.translate_text(raw, &repair_prompt))
        .await;
    match res {
        Ok(s) => Some(s),
        Err(e) => {
            if verbose {
                println!("  [!] JSON repair failed ({}).", e);
            } else {
                eprintln!("  [!] JSON repair failed ({}).", e);
            }
            None
        }
    }
}

/// Loads a glossary file: JSON object mapping source term -> required target
/// term, e.g. `{"Luffy": "Luffy", "海賊王": "Raja Bajak Laut"}`.
///
/// Validation (caller should exit 2 on error): object required, keys/values
/// must be non-empty strings, max 500 entries, no case-insensitive duplicates.
pub fn load_glossary(path: &std::path::Path) -> Result<std::collections::BTreeMap<String, String>> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read glossary {:?}", path))?;
    let v: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse glossary {:?}", path))?;
    let obj = v.as_object().with_context(|| {
        format!(
            "Invalid glossary {:?}: top level must be a JSON object of term -> translation",
            path
        )
    })?;
    if obj.len() > 500 {
        bail!(
            "Invalid glossary {:?}: max 500 entries, got {}",
            path,
            obj.len()
        );
    }
    let mut out = std::collections::BTreeMap::new();
    let mut seen_lower = std::collections::HashSet::new();
    for (k, val) in obj {
        let dst = val.as_str().with_context(|| {
            format!(
                "Invalid glossary {:?}: value for {:?} must be a string",
                path, k
            )
        })?;
        if k.trim().is_empty() || dst.trim().is_empty() {
            bail!(
                "Invalid glossary {:?}: empty term or translation near {:?}",
                path,
                k
            );
        }
        if !seen_lower.insert(k.to_lowercase()) {
            bail!(
                "Invalid glossary {:?}: duplicate term (case-insensitive) {:?}",
                path,
                k
            );
        }
        out.insert(k.clone(), dst.to_string());
    }
    if out.is_empty() {
        bail!("Invalid glossary {:?}: no entries", path);
    }
    Ok(out)
}

fn contains_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// Enforces glossary compliance on `translations` in place.
///
/// Returns `(hits, misses)`: compliant (term, bubble) pairs vs. pairs where
/// the source term leaked through and a single rewrite retry failed.
pub async fn enforce_glossary(
    chain: &ProviderChain,
    limiter: &RateLimiter,
    translations: &mut HashMap<String, String>,
    glossary: &std::collections::BTreeMap<String, String>,
    target_lang: &str,
    verbose: bool,
) -> (usize, usize) {
    let mut hits = 0usize;
    let mut misses = 0usize;
    // Deterministic order for stable logs.
    let mut ids: Vec<String> = translations.keys().cloned().collect();
    ids.sort();
    for id in ids {
        let text = translations.get(&id).cloned().unwrap_or_default();
        if text.to_uppercase() == "SKIP" || text.trim().is_empty() {
            continue;
        }
        for (src, dst) in glossary.iter() {
            if contains_insensitive(&text, src) && !text.contains(dst) {
                let rewrite_prompt = format!(
                    "Rewrite the following {target_lang} manga dialogue so that every occurrence of \"{src}\" becomes \"{dst}\". Keep everything else identical. Output ONLY the rewritten text, no quotes, no commentary.\n\n{text}"
                );
                let mut fixed = false;
                for prov in chain.all_providers() {
                    match limiter
                        .execute_with_retry(verbose, || prov.translate_text(&text, &rewrite_prompt))
                        .await
                    {
                        Ok(s) if !s.trim().is_empty() => {
                            translations.insert(id.clone(), s.trim().to_string());
                            fixed = true;
                            break;
                        }
                        _ => continue,
                    }
                }
                let now = translations.get(&id).cloned().unwrap_or_default();
                if fixed && (!contains_insensitive(&now, src) || now.contains(dst)) {
                    hits += 1;
                } else {
                    eprintln!(
                        "  [Glossary] miss: term {:?} leaked in #{} even after rewrite",
                        src, id
                    );
                    misses += 1;
                }
            } else {
                hits += 1;
            }
        }
    }
    (hits, misses)
}

#[cfg(test)]
mod glossary_tests {
    use super::*;

    #[test]
    fn prompt_contains_glossary_block() {
        let mut g = std::collections::BTreeMap::new();
        g.insert("Luffy".to_string(), "Luffy".to_string());
        let p = build_translation_prompt_with_glossary("Indonesian", Some(&g));
        assert!(p.contains("GLOSSARY (must obey)"));
        assert!(p.contains("\"Luffy\" -> \"Luffy\""));
        let plain = build_translation_prompt("Indonesian");
        assert!(!plain.contains("GLOSSARY"));
    }

    #[test]
    fn load_glossary_rejects_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "[1,2]").unwrap();
        assert!(load_glossary(&bad).is_err());
        let empty = dir.path().join("empty.json");
        std::fs::write(&empty, "{}").unwrap();
        assert!(load_glossary(&empty).is_err());
        let dup = dir.path().join("dup.json");
        std::fs::write(&dup, r#"{"Luffy": "Luffy", "luffy": "Luffy"}"#).unwrap();
        assert!(load_glossary(&dup).is_err());
        let ok = dir.path().join("ok.json");
        std::fs::write(&ok, r#"{"Luffy": "Luffy"}"#).unwrap();
        let g = load_glossary(&ok).unwrap();
        assert_eq!(g.get("Luffy").unwrap(), "Luffy");
    }
}
