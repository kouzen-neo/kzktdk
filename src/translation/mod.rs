use anyhow::{Context, Result, bail};
use image::{DynamicImage, Rgb, RgbImage};
use reqwest::header::{HeaderMap, HeaderValue};
use std::collections::HashMap;
use std::io::Cursor;

use crate::model::yolo::Detection;
use crate::typesetting::Typesetter;

pub struct CropItem {
    pub id: String,
    pub image: RgbImage,
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

    format!(
        r#"You are an accurate, natural manga and comic translator translating into {target_lang}.
The image contains speech bubbles arranged vertically.
Each bubble is prefixed with a LARGE RED NUMBER on its left as its ID.

MAIN TASK:
Read the dialogue in each bubble and translate it faithfully into {target_lang}.

RULES:
1. Natural flow: Use natural conversational manga dialogue suitable for speech bubbles.{id_extra}
2. Honorifics: Keep Japanese honorifics (san, kun, chan, sama, senpai, etc.) as-is.
3. If a bubble contains only sound effects (SFX) or unreadable art, reply with "SKIP".
4. Return a valid JSON object mapping every visible red number to its translation.
5. Example: {example}

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

impl Provider {
    pub async fn translate_mosaic(
        &self,
        mosaic: &DynamicImage,
        prompt: &str,
    ) -> Result<HashMap<String, String>> {
        // Encode image to base64 JPEG
        let mut jpeg_bytes = Vec::new();
        mosaic.write_to(&mut Cursor::new(&mut jpeg_bytes), image::ImageFormat::Jpeg)?;
        let base64_image =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &jpeg_bytes);

        let client = reqwest::Client::new();

        let response_text = match self {
            Provider::Gemini { api_key, model } => {
                let url = format!(
                    "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                    model, api_key
                );

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
                        "temperature": 0.2,
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
                    "temperature": 0.2,
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
                let url = "https://api.anthropic.com/v1/messages";

                let body = serde_json::json!({
                    "model": model,
                    "max_tokens": 1024,
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
                headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));

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

        parse_translation_json(&response_text)
    }
}
