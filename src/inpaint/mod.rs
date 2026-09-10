use anyhow::{Context, Result};
use image::{GrayImage, Luma, RgbImage};
use inpaint::prelude::ImageInpaint;
use std::collections::VecDeque;

use crate::model::yolo::Detection;

pub const MIN_INTERIOR_SCORE_RATIO: f32 = 0.04;
pub const MIN_COMPONENT_AREA: usize = 10;
pub const MORPH_CLOSE_MIN_SIZE: f32 = 15.0;
pub const MORPH_CLOSE_MAX_SIZE: f32 = 45.0;

#[derive(Debug, Clone)]
struct Component {
    pixels: Vec<(u32, u32)>,
    area: usize,
    min_x: u32,
    max_x: u32,
    min_y: u32,
    max_y: u32,
    cx: f32,
    cy: f32,
}

/// Computes connected components on a binary mask (true = active pixels).
fn find_components(mask: &GrayImage, active_val: u8) -> Vec<Component> {
    let (w, h) = mask.dimensions();
    let mut visited = vec![false; (w * h) as usize];
    let mut components = Vec::new();

    for y in 0..h {
        for x in 0..w {
            let idx = (y * w + x) as usize;
            if visited[idx] || mask.get_pixel(x, y)[0] != active_val {
                continue;
            }

            let mut pixels = Vec::new();
            let mut queue = VecDeque::new();
            queue.push_back((x, y));
            visited[idx] = true;

            let mut min_x = x;
            let mut max_x = x;
            let mut min_y = y;
            let mut max_y = y;
            let mut sum_x = 0u64;
            let mut sum_y = 0u64;

            while let Some((cx, cy)) = queue.pop_front() {
                pixels.push((cx, cy));
                min_x = min_x.min(cx);
                max_x = max_x.max(cx);
                min_y = min_y.min(cy);
                max_y = max_y.max(cy);
                sum_x += cx as u64;
                sum_y += cy as u64;

                let neighbors = [
                    (cx.wrapping_sub(1), cy.wrapping_sub(1)),
                    (cx, cy.wrapping_sub(1)),
                    (cx + 1, cy.wrapping_sub(1)),
                    (cx.wrapping_sub(1), cy),
                    (cx + 1, cy),
                    (cx.wrapping_sub(1), cy + 1),
                    (cx, cy + 1),
                    (cx + 1, cy + 1),
                ];

                for (nx, ny) in neighbors {
                    if nx < w && ny < h {
                        let nidx = (ny * w + nx) as usize;
                        if !visited[nidx] && mask.get_pixel(nx, ny)[0] == active_val {
                            visited[nidx] = true;
                            queue.push_back((nx, ny));
                        }
                    }
                }
            }

            let area = pixels.len();
            if area > 0 {
                let count = area as f32;
                components.push(Component {
                    pixels,
                    area,
                    min_x,
                    max_x,
                    min_y,
                    max_y,
                    cx: sum_x as f32 / count,
                    cy: sum_y as f32 / count,
                });
            }
        }
    }

    components
}

/// Dilates a binary mask with an elliptical/circle radius.
fn dilate_mask(mask: &GrayImage, radius: i32) -> GrayImage {
    let (w, h) = mask.dimensions();
    let mut dilated = GrayImage::new(w, h);

    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if mask.get_pixel(x as u32, y as u32)[0] == 255 {
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        if dx * dx + dy * dy <= radius * radius {
                            let nx = x + dx;
                            let ny = y + dy;
                            if nx >= 0 && nx < w as i32 && ny >= 0 && ny < h as i32 {
                                dilated.put_pixel(nx as u32, ny as u32, Luma([255]));
                            }
                        }
                    }
                }
            }
        }
    }

    dilated
}

/// Erodes a binary mask using mathematical morphology duality: erode(M) = ~dilate(~M).
fn erode_mask(mask: &GrayImage, radius: i32) -> GrayImage {
    let (w, h) = mask.dimensions();
    let mut inverted = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            if mask.get_pixel(x, y)[0] == 0 {
                inverted.put_pixel(x, y, Luma([255]));
            }
        }
    }

    let dilated_inv = dilate_mask(&inverted, radius);
    let mut eroded = GrayImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            if dilated_inv.get_pixel(x, y)[0] == 0 {
                eroded.put_pixel(x, y, Luma([255]));
            }
        }
    }

    eroded
}

/// Morphological Close: Dilates then Erodes to bridge gaps while preserving original outer boundaries.
fn morph_close(mask: &GrayImage, radius: i32) -> GrayImage {
    let dilated = dilate_mask(mask, radius);
    erode_mask(&dilated, radius)
}

/// Inpaints a single bubble crop matching the KZKT OpenCV ImageInpainting algorithm.
pub fn inpaint_crop(crop: &mut RgbImage) -> Result<()> {
    inpaint_crop_with_mask(crop, None)
}

/// Inpaints a crop, OR-ing an optional custom brush mask (white = inpaint)
/// into the auto-detected text mask. `extra` must match crop dimensions.
pub fn inpaint_crop_with_mask(crop: &mut RgbImage, extra: Option<&GrayImage>) -> Result<()> {
    let (cols, rows) = crop.dimensions();
    if cols < 4 || rows < 4 {
        return Ok(());
    }

    let total_pixels = (cols * rows) as f32;

    // 1. Calculate average luminance to adapt between light and dark bubbles
    let mut sum_lum = 0.0f64;
    for y in 0..rows {
        for x in 0..cols {
            let p = crop.get_pixel(x, y);
            sum_lum += 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64;
        }
    }
    let mean_lum = (sum_lum / total_pixels as f64) as f32;
    let is_light = mean_lum > 128.0;

    let mut bg_mask = GrayImage::new(cols, rows);
    let mut text_candidate_mask = GrayImage::new(cols, rows);

    for y in 0..rows {
        for x in 0..cols {
            let p = crop.get_pixel(x, y);
            let lum =
                (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32).round() as u8;

            if is_light {
                // Light bubble: Background >= 195, Text <= 180
                if lum >= 195 {
                    bg_mask.put_pixel(x, y, Luma([255]));
                }
                if lum <= 180 {
                    text_candidate_mask.put_pixel(x, y, Luma([255]));
                }
            } else {
                // Dark bubble: Background <= 60, Text >= 75
                if lum <= 60 {
                    bg_mask.put_pixel(x, y, Luma([255]));
                }
                if lum >= 75 {
                    text_candidate_mask.put_pixel(x, y, Luma([255]));
                }
            }
        }
    }

    // 2. Isolate the true interior of the bubble using central component scoring
    let bg_components = find_components(&bg_mask, 255);
    let center_x = cols as f32 / 2.0;
    let center_y = rows as f32 / 2.0;
    let max_dist = (center_x.hypot(center_y) + 1.0).max(1.0);

    let mut best_bg: Option<&Component> = None;
    let mut max_interior_score = -1.0f32;

    for comp in &bg_components {
        if comp.area < MIN_COMPONENT_AREA {
            continue;
        }
        let dist = (comp.cx - center_x).hypot(comp.cy - center_y);
        let score = comp.area as f32 * (1.0 - (dist / max_dist) * 0.5);
        if score > max_interior_score {
            max_interior_score = score;
            best_bg = Some(comp);
        }
    }

    let mut bubble_interior = GrayImage::new(cols, rows);

    if let Some(comp) = best_bg
        && max_interior_score >= total_pixels * MIN_INTERIOR_SCORE_RATIO
    {
        let mut single_bg = GrayImage::new(cols, rows);
        for &(px, py) in &comp.pixels {
            single_bg.put_pixel(px, py, Luma([255]));
        }

        // In KZKT: closeSize = maxOf(15.0, minOf(45.0, (minOf(cols, rows) / 5.0) * 2.0 + 1.0))
        // radius = closeSize / 2
        let close_size = MORPH_CLOSE_MIN_SIZE
            .max(MORPH_CLOSE_MAX_SIZE.min((cols.min(rows) as f32 / 5.0) * 2.0 + 1.0));
        let close_radius = (close_size / 2.0).round() as i32;

        // TRUE morphological closing (dilate followed by erode)
        bubble_interior = morph_close(&single_bg, close_radius);
    }

    // Fallback: If no single background component was found, use an inset interior box
    if max_interior_score < total_pixels * MIN_INTERIOR_SCORE_RATIO {
        let inset_x = (cols as f32 * 0.10).round().max(3.0) as u32;
        let inset_y = (rows as f32 * 0.10).round().max(3.0) as u32;
        if cols > inset_x * 2 && rows > inset_y * 2 {
            for y in inset_y..rows.saturating_sub(inset_y) {
                for x in inset_x..cols.saturating_sub(inset_x) {
                    bubble_interior.put_pixel(x, y, Luma([255]));
                }
            }
        }
    }

    // 3. Exclude outer borders, panel frames, and outside artwork from text candidates
    // Faithfully implementing KZKT edgeTouchCount logic
    let text_components = find_components(&text_candidate_mask, 255);
    let mut valid_text_mask = GrayImage::new(cols, rows);

    for comp in &text_components {
        if comp.area < 2 {
            continue;
        }

        let w = comp.max_x - comp.min_x + 1;
        let h = comp.max_y - comp.min_y + 1;

        let spans_crop = w >= (cols as f32 * 0.78) as u32 && h >= (rows as f32 * 0.78) as u32;

        let mut edge_touch_count = 0;
        if comp.min_x <= 1 {
            edge_touch_count += 1;
        }
        if comp.min_y <= 1 {
            edge_touch_count += 1;
        }
        if comp.max_x >= cols.saturating_sub(2) {
            edge_touch_count += 1;
        }
        if comp.max_y >= rows.saturating_sub(2) {
            edge_touch_count += 1;
        }

        let touches_edge = edge_touch_count > 0;
        let is_border_or_outer_art = spans_crop
            || (touches_edge
                && (w >= (cols as f32 * 0.35) as u32
                    || h >= (rows as f32 * 0.35) as u32
                    || comp.area as f32 > total_pixels * 0.07))
            || (edge_touch_count >= 2
                && (w >= (cols as f32 * 0.25) as u32
                    || h >= (rows as f32 * 0.25) as u32
                    || comp.area as f32 > total_pixels * 0.04))
            || (comp.area as f32 > total_pixels * 0.35);

        if !is_border_or_outer_art {
            for &(px, py) in &comp.pixels {
                valid_text_mask.put_pixel(px, py, Luma([255]));
            }
        }
    }

    // 4. Double constraint with 3x3 (radius 1) anti-aliasing dilation
    let mut text_mask = GrayImage::new(cols, rows);
    for y in 0..rows {
        for x in 0..cols {
            if valid_text_mask.get_pixel(x, y)[0] == 255
                && bubble_interior.get_pixel(x, y)[0] == 255
            {
                text_mask.put_pixel(x, y, Luma([255]));
            }
        }
    }

    // Dilate text mask with 3x3 structuring element (radius 1)
    let dilated_text = dilate_mask(&text_mask, 1);

    // Re-apply interior constraint so dilation never crosses the bubble outline
    let mut final_mask = GrayImage::new(cols, rows);
    let mut has_masked_pixels = false;
    for y in 1..rows.saturating_sub(1) {
        for x in 1..cols.saturating_sub(1) {
            if dilated_text.get_pixel(x, y)[0] == 255 && bubble_interior.get_pixel(x, y)[0] == 255 {
                final_mask.put_pixel(x, y, Luma([255]));
                has_masked_pixels = true;
            }
        }
    }

    // OR in the custom brush mask (GUI) when provided.
    if let Some(custom) = extra {
        if custom.dimensions() != (cols, rows) {
            anyhow::bail!(
                "Custom mask size {:?} does not match crop {}x{}",
                custom.dimensions(),
                cols,
                rows
            );
        }
        for y in 0..rows {
            for x in 0..cols {
                if custom.get_pixel(x, y)[0] > 128 {
                    final_mask.put_pixel(x, y, Luma([255]));
                    has_masked_pixels = true;
                }
            }
        }
    }

    if std::env::var("DEBUG_INPAINT").is_ok() {
        let _ = crop.save("/tmp/debug_0_crop.png");
        let _ = bg_mask.save("/tmp/debug_1_bg_mask.png");
        let _ = bubble_interior.save("/tmp/debug_2_bubble_interior.png");
        let _ = valid_text_mask.save("/tmp/debug_3_valid_text.png");
        let _ = final_mask.save("/tmp/debug_4_final_mask.png");
    }

    if !has_masked_pixels {
        return Ok(());
    }

    // 5. Run native Fast Marching Telea inpainting on the crop
    crop.telea_inpaint(&final_mask, 3)
        .map_err(|e| anyhow::anyhow!("Telea inpainting error: {:?}", e))?;

    Ok(())
}

/// Inpaints all detected speech bubbles in the provided image.
/// Parallelized with rayon (like KZKT ImageInpainting.inpaintTranslated parallel coroutines).
pub fn inpaint_image(img: &mut RgbImage, detections: &[Detection]) -> Result<()> {
    let regions: Vec<MaskedRegion> = detections
        .iter()
        .map(|d| MaskedRegion {
            det: d.clone(),
            mask: None,
        })
        .collect();
    inpaint_regions(img, &regions)
}

/// A bubble region with an optional custom brush mask (white = inpaint).
/// When the mask is present it must match the detection crop size exactly.
#[derive(Debug, Clone)]
pub struct MaskedRegion {
    pub det: Detection,
    pub mask: Option<GrayImage>,
}

/// Loads a custom brush mask file and validates it against the crop size.
/// Accepts any image format readable by the `image` crate; pixels with
/// luminance > 128 count as masked.
pub fn load_mask_for(path: &std::path::Path, w: u32, h: u32) -> Result<GrayImage> {
    let img = image::open(path).with_context(|| format!("Failed to open mask {:?}", path))?;
    let gray = img.to_luma8();
    if gray.dimensions() != (w, h) {
        anyhow::bail!(
            "Mask {:?} size {:?} does not match bubble crop {}x{}",
            path,
            gray.dimensions(),
            w,
            h
        );
    }
    Ok(gray)
}

/// Inpaints regions with optional per-region custom masks, in parallel.
pub fn inpaint_regions(img: &mut RgbImage, regions: &[MaskedRegion]) -> Result<()> {
    use rayon::prelude::*;

    let (orig_w, orig_h) = img.dimensions();

    // Extract crops + rects for parallel processing
    struct Task {
        x1: u32,
        y1: u32,
        w: u32,
        h: u32,
        crop: RgbImage,
        mask: Option<GrayImage>,
    }

    let mut tasks: Vec<Task> = Vec::new();
    for region in regions {
        let det = &region.det;
        let x1 = det.x1.min(orig_w.saturating_sub(1));
        let y1 = det.y1.min(orig_h.saturating_sub(1));
        let x2 = det.x2.min(orig_w);
        let y2 = det.y2.min(orig_h);

        let w = x2.saturating_sub(x1);
        let h = y2.saturating_sub(y1);

        if w < 4 || h < 4 {
            continue;
        }

        if let Some(ref m) = region.mask {
            if m.dimensions() != (w, h) {
                anyhow::bail!(
                    "Custom mask size {:?} does not match bubble crop {}x{}",
                    m.dimensions(),
                    w,
                    h
                );
            }
        }

        let mut crop = RgbImage::new(w, h);
        for cy in 0..h {
            for cx in 0..w {
                crop.put_pixel(cx, cy, *img.get_pixel(x1 + cx, y1 + cy));
            }
        }
        tasks.push(Task {
            x1,
            y1,
            w,
            h,
            crop,
            mask: region.mask.clone(),
        });
    }

    if tasks.is_empty() {
        return Ok(());
    }

    // Parallel inpaint (CPU-bound). Custom-mask size was validated above,
    // so per-crop errors here are unexpected; keep first error.
    let first_err = std::sync::Mutex::new(None::<String>);
    tasks.par_iter_mut().for_each(|t| {
        if let Err(e) = inpaint_crop_with_mask(&mut t.crop, t.mask.as_ref()) {
            *first_err.lock().unwrap() = Some(e.to_string());
        }
    });
    if let Some(msg) = first_err.into_inner().unwrap() {
        anyhow::bail!("Inpainting failed: {}", msg);
    }

    // Sequential copy back (needs &mut img)
    for t in tasks {
        for cy in 0..t.h {
            for cx in 0..t.w {
                img.put_pixel(t.x1 + cx, t.y1 + cy, *t.crop.get_pixel(cx, cy));
            }
        }
    }

    Ok(())
}
