pub mod hyphenator;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use anyhow::{Context, Result};
use image::{Rgb, RgbImage};
use std::collections::VecDeque;

use crate::model::yolo::Detection;
use hyphenator::Hyphenator;

pub const ELLIPTICAL_CURVE_FACTOR: f32 = 0.40;
pub const ELLIPTICAL_MIN_SCALE: f32 = 0.82;
pub const ELLIPTICAL_MIN_BUDGET: f32 = 0.20;
pub const STROKE_RADIUS_DIVISOR: f32 = 11.0;

#[derive(Debug, Clone, Copy)]
pub struct TextSettings {
    scale_w: f32,
    scale_h: f32,
    font_scale: f32,
    spacing_ratio: f32,
    max_font: f32,
    min_font: f32,
}

pub struct Typesetter<'a> {
    latin_font: FontRef<'a>,
    cjk_font: Option<FontRef<'a>>,
}

impl<'a> Typesetter<'a> {
    pub fn new(latin_data: &'a [u8], cjk_data: Option<&'a [u8]>) -> Result<Self> {
        let latin_font =
            FontRef::try_from_slice(latin_data).context("Failed to parse latin font")?;
        let cjk_font = if let Some(data) = cjk_data {
            Some(FontRef::try_from_slice(data).context("Failed to parse CJK font")?)
        } else {
            None
        };

        Ok(Self {
            latin_font,
            cjk_font,
        })
    }

    pub fn has_non_latin(text: &str) -> bool {
        text.chars().any(|c| {
            matches!(c,
                '\u{3040}'..='\u{309f}' | // Hiragana
                '\u{30a0}'..='\u{30ff}' | // Katakana
                '\u{4e00}'..='\u{9fff}' | // CJK Unified Ideographs
                '\u{ac00}'..='\u{d7af}' | // Hangul
                '\u{0e00}'..='\u{0e7f}' | // Thai
                '\u{0400}'..='\u{04ff}' | // Cyrillic
                '\u{0600}'..='\u{06ff}'   // Arabic
            )
        })
    }

    fn select_font(&self, text: &str) -> &FontRef<'a> {
        if Self::has_non_latin(text)
            && let Some(ref cjk) = self.cjk_font
        {
            return cjk;
        }
        &self.latin_font
    }

    fn pick_text_settings(box_w: f32, box_h: f32, text: &str) -> TextSettings {
        let clean_len = text.chars().filter(|c| !c.is_whitespace()).count();
        let area = box_w * box_h;

        let is_large_bubble = box_w >= 150.0 && box_h >= 130.0 && area >= 30000.0;
        let is_short = clean_len <= 55;
        let is_very_short = clean_len <= 28;

        if is_large_bubble && is_very_short {
            TextSettings {
                scale_w: 0.78,
                scale_h: 0.76,
                font_scale: 0.88,
                spacing_ratio: 0.18,
                max_font: 76.0,
                min_font: 10.0,
            }
        } else if is_large_bubble && is_short {
            TextSettings {
                scale_w: 0.75,
                scale_h: 0.74,
                font_scale: 0.86,
                spacing_ratio: 0.18,
                max_font: 72.0,
                min_font: 10.0,
            }
        } else {
            TextSettings {
                scale_w: 0.72,
                scale_h: 0.72,
                font_scale: 0.84,
                spacing_ratio: 0.16,
                max_font: 68.0,
                min_font: 8.0,
            }
        }
    }

    /// Calculates safe elliptical width for line [line_idx] out of [total_lines]
    /// to fit comic speech bubbles naturally (diamond-shape wrapping).
    pub fn get_elliptical_line_width(max_w: f32, line_idx: usize, total_lines: usize) -> f32 {
        if total_lines <= 1 {
            return max_w;
        }
        let normalized_y = (2.0 * line_idx as f32 + 1.0) / total_lines as f32 - 1.0;
        let clamped_y = normalized_y.clamp(-0.85, 0.85);
        let h_scale = (1.0 - (clamped_y * clamped_y) * ELLIPTICAL_CURVE_FACTOR)
            .max(ELLIPTICAL_MIN_BUDGET)
            .sqrt()
            .clamp(ELLIPTICAL_MIN_SCALE, 1.0);
        max_w * h_scale
    }

    pub fn measure_text(&self, text: &str, scale: PxScale) -> f32 {
        let font = self.select_font(text);
        let scaled_font = font.as_scaled(scale);
        let mut width = 0.0f32;
        let mut last_glyph = None;

        for ch in text.chars() {
            let gid = font.glyph_id(ch);
            if let Some(last) = last_glyph {
                width += scaled_font.kern(last, gid);
            }
            width += scaled_font.h_advance(gid);
            last_glyph = Some(gid);
        }

        width
    }

    /// Wraps text into lines constrained by maxWidth with hyphenation and diamond shape.
    pub fn wrap_text_with_hyphenation(
        &self,
        text: &str,
        scale: PxScale,
        max_w: f32,
        target_language: Option<&str>,
    ) -> Vec<String> {
        if text.is_empty() {
            return Vec::new();
        }

        let is_cjk = Self::has_non_latin(text) && !text.contains(' ');
        if is_cjk {
            let chars: Vec<char> = text.chars().collect();
            let mut lines = Vec::new();
            let mut current_line = String::new();

            for ch in chars {
                let candidate = format!("{}{}", current_line, ch);
                if self.measure_text(&candidate, scale) <= max_w || current_line.is_empty() {
                    current_line.push(ch);
                } else {
                    lines.push(current_line);
                    current_line = ch.to_string();
                }
            }
            if !current_line.is_empty() {
                lines.push(current_line);
            }
            return lines;
        }

        let paragraphs = text.split('\n');
        let mut all_formatted_lines = Vec::new();

        for paragraph in paragraphs {
            let raw_words: Vec<&str> = paragraph.split_whitespace().collect();
            if raw_words.is_empty() {
                continue;
            }

            let total_raw_chars: usize = paragraph.chars().count();
            let avg_char_w = self.measure_text("A", scale).max(1.0);
            let estimated_lines =
                ((total_raw_chars as f32 * avg_char_w / max_w.max(1.0)).round() as usize).max(1);

            let mut word_queue: VecDeque<String> =
                raw_words.into_iter().map(|w| w.to_string()).collect();
            let mut lines = Vec::new();
            let mut current_line = String::new();
            let mut hyphens_used = 0;

            while let Some(word) = word_queue.pop_front() {
                let candidate = if current_line.is_empty() {
                    word.clone()
                } else {
                    format!("{} {}", current_line, word)
                };

                let cand_w = self.measure_text(&candidate, scale);
                let line_limit =
                    Self::get_elliptical_line_width(max_w, lines.len(), estimated_lines);

                if cand_w <= line_limit {
                    current_line = candidate;
                } else if current_line.is_empty() {
                    // Single long word on an empty line: hyphenate
                    let candidates = if hyphens_used < 2 {
                        Hyphenator::get_hyphenation_candidates(&word, target_language)
                    } else {
                        Vec::new()
                    };

                    let mut split = false;
                    for (prefix, suffix) in candidates {
                        if self.measure_text(&prefix, scale) <= line_limit {
                            lines.push(prefix);
                            word_queue.push_front(suffix);
                            hyphens_used += 1;
                            split = true;
                            break;
                        }
                    }

                    if !split {
                        lines.push(word);
                    }
                } else {
                    // Current line has content: push and defer word
                    lines.push(current_line);
                    current_line = String::new();
                    word_queue.push_front(word);
                }
            }

            if !current_line.is_empty() {
                lines.push(current_line);
            }

            all_formatted_lines.extend(lines);
        }

        all_formatted_lines
    }

    /// Binary search for optimal font size fitting inside the bubble box.
    pub fn fit_text(
        &self,
        display_text: &str,
        box_w: f32,
        box_h: f32,
        settings: TextSettings,
        target_language: Option<&str>,
    ) -> (f32, f32, Vec<String>) {
        let max_w = box_w * settings.scale_w;
        let max_h = box_h * settings.scale_h;
        let min_font = settings.min_font;
        let max_font = settings.max_font;

        let mut low = min_font;
        let mut high = max_font;
        let mut best_font_size = min_font;
        let mut best_spacing = 1.0f32;

        let font = self.select_font(display_text);

        while low <= high {
            let f_size = ((low + high) / 2.0).round();
            let scale = PxScale::from(f_size);
            let scaled_font = font.as_scaled(scale);

            let cap_h = font
                .outline_glyph(scaled_font.scaled_glyph('H'))
                .map(|g| g.px_bounds().height())
                .unwrap_or(f_size * 0.72);
            let spacing = (f_size * settings.spacing_ratio).round().max(1.0);
            let lines =
                self.wrap_text_with_hyphenation(display_text, scale, max_w, target_language);

            let mut max_line_w = 0.0f32;
            for line in &lines {
                let lw = self.measure_text(line, scale);
                if lw > max_line_w {
                    max_line_w = lw;
                }
            }

            let total_h = if !lines.is_empty() {
                (lines.len() - 1) as f32 * (f_size + spacing) + cap_h
            } else {
                0.0
            };

            if max_line_w <= max_w && total_h <= max_h {
                best_font_size = f_size;
                best_spacing = spacing;
                low = f_size + 1.0;
            } else {
                high = f_size - 1.0;
            }
        }

        // Apply final font scale factor
        best_font_size = (best_font_size * settings.font_scale).max(min_font);
        let final_scale = PxScale::from(best_font_size);
        let final_lines =
            self.wrap_text_with_hyphenation(display_text, final_scale, max_w, target_language);

        (best_font_size, best_spacing, final_lines)
    }

    /// Renders wrapped text centered inside a speech bubble with clean outline stroke and optical centering.
    pub fn render_bubble_text(
        &self,
        img: &mut RgbImage,
        detection: &Detection,
        text: &str,
        target_language: Option<&str>,
        force_text_color: Option<Rgb<u8>>,
    ) {
        self.render_bubble_text_with_style(
            img,
            detection,
            text,
            target_language,
            force_text_color,
            None,
        )
    }

    pub fn render_bubble_text_with_style(
        &self,
        img: &mut RgbImage,
        detection: &Detection,
        text: &str,
        target_language: Option<&str>,
        force_text_color: Option<Rgb<u8>>,
        bubble_style: Option<&crate::metadata::BubbleStyle>,
    ) {
        let is_non_latin = Self::has_non_latin(text);
        // Manga standard typesetting: Latin text is rendered in UPPERCASE
        let display_text = if !is_non_latin {
            text.to_uppercase()
        } else {
            text.to_string()
        };

        let box_w = detection.width() as f32;
        let box_h = detection.height() as f32;
        let settings = Self::pick_text_settings(box_w, box_h, &display_text);

        let (best_font_size, best_spacing, final_lines) = if let Some(style) = bubble_style {
            if let Some(fs) = style.font_size {
                let clamped = fs.clamp(settings.min_font, settings.max_font);
                let spacing = (clamped * settings.spacing_ratio).round().max(1.0);
                let scale = PxScale::from(clamped);
                let max_w = box_w * settings.scale_w;
                let lines =
                    self.wrap_text_with_hyphenation(&display_text, scale, max_w, target_language);
                (clamped, spacing, lines)
            } else {
                self.fit_text(&display_text, box_w, box_h, settings, target_language)
            }
        } else {
            self.fit_text(&display_text, box_w, box_h, settings, target_language)
        };

        if final_lines.is_empty() {
            return;
        }

        let font = self.select_font(&display_text);
        let scale = PxScale::from(best_font_size);
        let scaled_font = font.as_scaled(scale);
        let cap_h = font
            .outline_glyph(scaled_font.scaled_glyph('H'))
            .map(|g| g.px_bounds().height())
            .unwrap_or(best_font_size * 0.72);

        let num_lines = final_lines.len();
        let vertical_ratio = box_h / box_w.max(1.0);

        // Dynamic line spacing: if bubble is tall and has 2-5 lines, expand line gap slightly (20%)
        let effective_line_gap = if vertical_ratio >= 1.3 && (2..=5).contains(&num_lines) {
            best_spacing.max(best_font_size * 0.20)
        } else {
            best_spacing
        };

        let total_text_h = if num_lines > 0 {
            (num_lines - 1) as f32 * (best_font_size + effective_line_gap) + cap_h
        } else {
            0.0
        };

        // Pure optical vertical centering
        let start_y = detection.y1 as f32 + (box_h - total_text_h) / 2.0;

        // Auto determine text and stroke colors based on background, with style override
        let is_dark_bg = detection.is_dark_bg(img);
        let auto_text_color = if is_dark_bg {
            Rgb([255, 255, 255])
        } else {
            Rgb([18, 18, 18])
        };
        let text_color = if let Some(style) = bubble_style {
            if let Some(c) = style.text_color {
                Rgb(c)
            } else {
                force_text_color.unwrap_or(auto_text_color)
            }
        } else {
            force_text_color.unwrap_or(auto_text_color)
        };
        // Stroke override
        let stroke_color = if let Some(style) = bubble_style {
            if let Some(c) = style.stroke_color {
                Rgb(c)
            } else if text_color == Rgb([255, 255, 255]) {
                Rgb([0, 0, 0])
            } else {
                Rgb([255, 255, 255])
            }
        } else if text_color == Rgb([255, 255, 255]) {
            Rgb([0, 0, 0])
        } else {
            Rgb([255, 255, 255])
        };

        // Proportional stroke radius: clamp(1.0, 3.5, best_font_size / STROKE_RADIUS_DIVISOR)
        let stroke_radius = (best_font_size / STROKE_RADIUS_DIVISOR).clamp(1.0, 3.5);
        let pad = (stroke_radius + 4.0).ceil() as i32;

        let buf_w = (box_w as i32 + pad * 2).max(1) as usize;
        let buf_h = (box_h as i32 + pad * 2).max(1) as usize;
        let mut text_alpha = vec![0.0f32; buf_w * buf_h];

        // 1. Render all glyphs to local text_alpha buffer
        for (idx, line) in final_lines.iter().enumerate() {
            let line_w = self.measure_text(line, scale);
            let line_x = if let Some(style) = bubble_style {
                match style.align.as_deref() {
                    Some("left") => 0.0,
                    Some("right") => box_w - line_w,
                    _ => (box_w - line_w) / 2.0,
                }
            } else {
                (box_w - line_w) / 2.0
            };
            let baseline_rel_y = (start_y - detection.y1 as f32)
                + idx as f32 * (best_font_size + effective_line_gap)
                + cap_h;

            let mut curr_x = line_x + pad as f32;
            let target_y = baseline_rel_y + pad as f32;
            let mut last_glyph = None;

            for ch in line.chars() {
                let gid = font.glyph_id(ch);
                if let Some(last) = last_glyph {
                    curr_x += scaled_font.kern(last, gid);
                }

                if let Some(qg) =
                    font.outline_glyph(scaled_font.glyph_with_target(ch, curr_x, target_y))
                {
                    let bounds = qg.px_bounds();
                    qg.draw(|gx, gy, c| {
                        let px = bounds.min.x as i32 + gx as i32;
                        let py = bounds.min.y as i32 + gy as i32;
                        if px >= 0 && (px as usize) < buf_w && py >= 0 && (py as usize) < buf_h {
                            let b_idx = py as usize * buf_w + px as usize;
                            text_alpha[b_idx] = text_alpha[b_idx].max(c);
                        }
                    });
                }

                curr_x += scaled_font.h_advance(gid);
                last_glyph = Some(gid);
            }
        }

        // 2. Generate smooth circular stroke coverage mask
        let r_ceil = stroke_radius.ceil() as i32;
        let r_sq = stroke_radius * stroke_radius;
        let mut stroke_alpha = vec![0.0f32; buf_w * buf_h];

        for by in 0..buf_h as i32 {
            for bx in 0..buf_w as i32 {
                let mut max_val = 0.0f32;
                for dy in -r_ceil..=r_ceil {
                    for dx in -r_ceil..=r_ceil {
                        let dist_sq = (dx * dx + dy * dy) as f32;
                        if dist_sq <= r_sq {
                            let nx = bx + dx;
                            let ny = by + dy;
                            if nx >= 0 && nx < buf_w as i32 && ny >= 0 && ny < buf_h as i32 {
                                let val = text_alpha[ny as usize * buf_w + nx as usize];
                                if val > max_val {
                                    max_val = val;
                                }
                            }
                        }
                    }
                }
                stroke_alpha[by as usize * buf_w + bx as usize] = max_val;
            }
        }

        // 3. Composite crisp text over stroke over image in a single pass
        let (img_w, img_h) = img.dimensions();
        for by in 0..buf_h as i32 {
            for bx in 0..buf_w as i32 {
                let idx = by as usize * buf_w + bx as usize;
                let a_text = text_alpha[idx];
                let a_stroke = stroke_alpha[idx];

                if a_text <= 0.001 && a_stroke <= 0.001 {
                    continue;
                }

                let img_x = detection.x1 as i32 - pad + bx;
                let img_y = detection.y1 as i32 - pad + by;

                if img_x < 0 || img_x >= img_w as i32 || img_y < 0 || img_y >= img_h as i32 {
                    continue;
                }

                let orig = img.get_pixel(img_x as u32, img_y as u32);
                let eff_text = a_text;
                let eff_stroke = a_stroke * (1.0 - a_text);
                let eff_bg = (1.0 - eff_text - eff_stroke).max(0.0);

                let r = (text_color[0] as f32 * eff_text
                    + stroke_color[0] as f32 * eff_stroke
                    + orig[0] as f32 * eff_bg)
                    .round() as u8;
                let g = (text_color[1] as f32 * eff_text
                    + stroke_color[1] as f32 * eff_stroke
                    + orig[1] as f32 * eff_bg)
                    .round() as u8;
                let b = (text_color[2] as f32 * eff_text
                    + stroke_color[2] as f32 * eff_stroke
                    + orig[2] as f32 * eff_bg)
                    .round() as u8;

                img.put_pixel(img_x as u32, img_y as u32, Rgb([r, g, b]));
            }
        }
    }
}

trait GlyphTargetHelper {
    fn glyph_with_target(&self, ch: char, x: f32, y: f32) -> ab_glyph::Glyph;
}

impl<F: Font> GlyphTargetHelper for ab_glyph::PxScaleFont<F> {
    fn glyph_with_target(&self, ch: char, x: f32, y: f32) -> ab_glyph::Glyph {
        let mut g = self.scaled_glyph(ch);
        g.position = ab_glyph::point(x, y);
        g
    }
}
