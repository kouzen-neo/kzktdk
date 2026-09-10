use crate::ocr::{OcrEngine, TextRegion};
use image::{Rgb, RgbImage};

const MAX_DETECT_DIM: u32 = 2048;

/// Check if point is inside any bbox
fn point_in_boxes(x: u32, y: u32, boxes: &[[u32; 4]]) -> bool {
    for b in boxes {
        if x >= b[0] && x < b[2] && y >= b[1] && y < b[3] {
            return true;
        }
    }
    false
}

fn iou(a: [u32; 4], b: [u32; 4]) -> f32 {
    let x1 = a[0].max(b[0]);
    let y1 = a[1].max(b[1]);
    let x2 = a[2].min(b[2]);
    let y2 = a[3].min(b[3]);
    if x2 <= x1 || y2 <= y1 {
        return 0.0;
    }
    let inter = (x2 - x1) as f32 * (y2 - y1) as f32;
    let area_a = (a[2] - a[0]) as f32 * (a[3] - a[1]) as f32;
    let area_b = (b[2] - b[0]) as f32 * (b[3] - b[1]) as f32;
    let union = area_a + area_b - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

/// Merge nearby boxes with gap threshold 20px (port ImageProcessor.mergeNearbyTextBoxes)
pub fn merge_nearby_text_boxes(mut boxes: Vec<[u32; 4]>, gap: u32) -> Vec<[u32; 4]> {
    if boxes.is_empty() {
        return boxes;
    }
    // Simple clustering: iteratively merge overlapping/close boxes
    boxes.sort_by_key(|b| (b[1], b[0]));
    let mut merged: Vec<[u32; 4]> = Vec::new();
    for b in boxes {
        if let Some(last) = merged.last_mut() {
            // expanded gap check: if b is within gap of last
            let close_x = !(b[0] > last[2] + gap || last[0] > b[2] + gap);
            let close_y = !(b[1] > last[3] + gap || last[1] > b[3] + gap);
            if close_x && close_y {
                last[0] = last[0].min(b[0]);
                last[1] = last[1].min(b[1]);
                last[2] = last[2].max(b[2]);
                last[3] = last[3].max(b[3]);
                continue;
            }
        }
        merged.push(b);
    }
    merged
}

/// Core freetext detection: mask bubbles WHITE, scale to MAX_DETECT_DIM, call ocr, scale back, filter.
pub fn detect_free_text(
    img: &RgbImage,
    bubble_boxes: &[[u32; 4]],
    ocr: &dyn OcrEngine,
) -> Vec<[u32; 4]> {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let max_dim = w.max(h);
    let scale = if max_dim > MAX_DETECT_DIM {
        MAX_DETECT_DIM as f32 / max_dim as f32
    } else {
        1.0
    };

    // Clone and mask bubbles white
    let mut masked = img.clone();
    for b in bubble_boxes {
        let x1 = b[0].min(w);
        let y1 = b[1].min(h);
        let x2 = b[2].min(w);
        let y2 = b[3].min(h);
        if x1 >= x2 || y1 >= y2 {
            continue;
        }
        for y in y1..y2 {
            for x in x1..x2 {
                masked.put_pixel(x, y, Rgb([255, 255, 255]));
            }
        }
    }
    // Scale if needed
    let (scaled, scaled_boxes): (RgbImage, Vec<[u32; 4]>) = if (scale - 1.0).abs() > f32::EPSILON {
        let nw = (w as f32 * scale).round() as u32;
        let nh = (h as f32 * scale).round() as u32;
        let scaled_img =
            image::imageops::resize(&masked, nw, nh, image::imageops::FilterType::Triangle);
        (scaled_img, Vec::new())
    } else {
        (masked, Vec::new())
    };
    let _ = scaled_boxes;

    // Call OCR — pass empty exclude since we already masked
    let regions: Vec<TextRegion> = ocr.recognize_regions(&scaled, &[]);

    // Scale back + filter
    let mut out: Vec<[u32; 4]> = Vec::new();
    for r in regions {
        let mut bbox = r.bbox;
        // scale back
        if (scale - 1.0).abs() > f32::EPSILON {
            let inv = 1.0 / scale;
            bbox[0] = ((bbox[0] as f32 * inv).round() as u32).min(w.saturating_sub(1));
            bbox[1] = ((bbox[1] as f32 * inv).round() as u32).min(h.saturating_sub(1));
            bbox[2] = ((bbox[2] as f32 * inv).round() as u32).min(w);
            bbox[3] = ((bbox[3] as f32 * inv).round() as u32).min(h);
        }
        // clamp
        bbox[0] = bbox[0].min(w);
        bbox[1] = bbox[1].min(h);
        bbox[2] = bbox[2].min(w);
        bbox[3] = bbox[3].min(h);
        if bbox[0] >= bbox[2] || bbox[1] >= bbox[3] {
            continue;
        }
        let bw = bbox[2] - bbox[0];
        let bh = bbox[3] - bbox[1];
        if bw < 12 || bh < 8 {
            continue;
        }
        let txt = r.text.trim().to_string();
        if txt.is_empty() {
            continue;
        }
        // single char alphanumeric/symbol skip (PagePreparer.kt:73)
        if txt.chars().count() == 1 {
            let c = txt.chars().next().unwrap();
            if c.is_ascii_alphanumeric() || ".,;:!?-_()[]{}\"'/\\|@#$%^&*+=<>`~".contains(c) {
                continue;
            }
        }
        // center inside bubble skip
        let cx = (bbox[0] + bbox[2]) / 2;
        let cy = (bbox[1] + bbox[3]) / 2;
        if point_in_boxes(cx, cy, bubble_boxes) {
            continue;
        }
        // IoU >=0.3 skip
        let mut overlap = false;
        for bb in bubble_boxes {
            if iou(bbox, *bb) >= 0.3 {
                overlap = true;
                break;
            }
        }
        if overlap {
            continue;
        }
        out.push(bbox);
    }

    merge_nearby_text_boxes(out, 20)
}

/// Helper to produce padded crop for freetext (6% pad) and bubble crops consistency
pub fn freetext_pad(bbox: [u32; 4], img_w: u32, img_h: u32) -> [u32; 4] {
    // 6% expansion
    let w = bbox[2] - bbox[0];
    let h = bbox[3] - bbox[1];
    let pad_x = ((w as f32 * 0.06).round() as u32).max(2);
    let pad_y = ((h as f32 * 0.06).round() as u32).max(2);
    let x1 = bbox[0].saturating_sub(pad_x);
    let y1 = bbox[1].saturating_sub(pad_y);
    let x2 = (bbox[2] + pad_x).min(img_w);
    let y2 = (bbox[3] + pad_y).min(img_h);
    [x1, y1, x2, y2]
}

/// Reading direction for [`detect_reading_order`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadingMode {
    /// Manga/manhwa: right column first, then top-to-bottom inside a column.
    R2L,
    /// Western comics: left column first, then top-to-bottom inside a column.
    L2R,
}

impl ReadingMode {
    pub fn from_key(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "r2l" | "rtl" | "manga" | "jp" | "kr" | "cn" => Some(Self::R2L),
            "l2r" | "ltr" | "western" | "comic" | "en" => Some(Self::L2R),
            _ => None,
        }
    }
}

/// Order bubble indices in natural reading order.
///
/// Groups boxes into columns by horizontal overlap, orders columns by
/// direction (R2L: right-to-left, L2R: left-to-right), then top-to-bottom
/// inside each column. Boxes spanning several columns join the one with the
/// largest overlap. Returns indices into `boxes`, most-important first.
pub fn detect_reading_order(boxes: &[[u32; 4]], mode: ReadingMode) -> Vec<usize> {
    let n = boxes.len();
    if n <= 1 {
        return (0..n).collect();
    }
    let cx = |b: &[u32; 4]| (b[0] as f32 + b[2] as f32) / 2.0;
    let cy = |b: &[u32; 4]| (b[1] as f32 + b[3] as f32) / 2.0;

    // Column gap threshold from median box width (robust to page scale).
    let mut widths: Vec<f32> = boxes
        .iter()
        .map(|b| (b[2].saturating_sub(b[0]) as f32).max(1.0))
        .collect();
    widths.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median_w = widths[n / 2];
    let col_gap = (median_w * 0.6).max(40.0);

    // Greedy column assignment by center-x, left to right.
    let mut by_cx: Vec<usize> = (0..n).collect();
    by_cx.sort_by(|&a, &b| {
        cx(&boxes[a])
            .partial_cmp(&cx(&boxes[b]))
            .unwrap()
            .then(a.cmp(&b))
    });
    // Each column: (min_x1, max_x2) range + member indices.
    let mut cols: Vec<(f32, f32, Vec<usize>)> = Vec::new();
    for &i in &by_cx {
        let c = cx(&boxes[i]);
        if let Some((_, _, members)) = cols.last_mut() {
            // New column when the center jumps beyond the gap from the
            // previous box center (columns are built left-to-right).
            let prev = *members.last().unwrap();
            if c - cx(&boxes[prev]) <= col_gap {
                members.push(i);
                continue;
            }
        }
        cols.push((boxes[i][0] as f32, boxes[i][2] as f32, vec![i]));
    }
    // Column ranges from members.
    for (x1, x2, members) in cols.iter_mut() {
        *x1 = members
            .iter()
            .map(|&i| boxes[i][0] as f32)
            .fold(f32::INFINITY, f32::min);
        *x2 = members
            .iter()
            .map(|&i| boxes[i][2] as f32)
            .fold(f32::NEG_INFINITY, f32::max);
    }
    // Reassign bridging boxes (spanning 2+ columns) to max-overlap column.
    if cols.len() > 1 {
        for ci in 0..cols.len() {
            let members = std::mem::take(&mut cols[ci].2);
            let mut keep = Vec::with_capacity(members.len());
            for i in members {
                let b = boxes[i];
                let w = (b[2].saturating_sub(b[0]) as f32).max(1.0);
                let mut best = ci;
                let mut best_ov = 0.0f32;
                let mut hits = 0;
                for (oi, (ox1, ox2, _)) in cols.iter().enumerate() {
                    let ov = (b[2] as f32).min(*ox2) - (b[0] as f32).max(*ox1);
                    if ov > w * 0.25 {
                        hits += 1;
                    }
                    if ov > best_ov {
                        best_ov = ov;
                        best = oi;
                    }
                }
                if hits >= 2 && best != ci {
                    cols[best].2.push(i);
                } else {
                    keep.push(i);
                }
            }
            cols[ci].2 = keep;
        }
        cols.retain(|(_, _, m)| !m.is_empty());
    }

    // Order columns by direction using mean center-x.
    let mut order: Vec<usize> = (0..cols.len()).collect();
    order.sort_by(|&a, &b| {
        let ma =
            cols[a].2.iter().map(|&i| cx(&boxes[i])).sum::<f32>() / cols[a].2.len().max(1) as f32;
        let mb =
            cols[b].2.iter().map(|&i| cx(&boxes[i])).sum::<f32>() / cols[b].2.len().max(1) as f32;
        match mode {
            ReadingMode::R2L => mb.partial_cmp(&ma).unwrap(),
            ReadingMode::L2R => ma.partial_cmp(&mb).unwrap(),
        }
    });

    let mut out = Vec::with_capacity(n);
    for ci in order {
        let mut members = std::mem::take(&mut cols[ci].2);
        members.sort_by(|&a, &b| {
            cy(&boxes[a])
                .partial_cmp(&cy(&boxes[b]))
                .unwrap()
                .then(a.cmp(&b))
        });
        out.extend(members);
    }
    out
}

#[cfg(test)]
mod reading_order_tests {
    use super::*;

    // 2x2 grid: TL=[0,0], TR=[1,0], BL=[0,1], BR=[1,1]
    fn grid() -> Vec<[u32; 4]> {
        vec![
            [10, 10, 100, 100],   // 0 TL
            [200, 10, 290, 100],  // 1 TR
            [10, 200, 100, 290],  // 2 BL
            [200, 200, 290, 290], // 3 BR
        ]
    }

    #[test]
    fn manga_r2l_reads_right_column_first() {
        assert_eq!(
            detect_reading_order(&grid(), ReadingMode::R2L),
            vec![1, 3, 0, 2]
        );
    }

    #[test]
    fn western_l2r_reads_left_column_first() {
        assert_eq!(
            detect_reading_order(&grid(), ReadingMode::L2R),
            vec![0, 2, 1, 3]
        );
    }

    #[test]
    fn bridging_title_joins_max_overlap_column() {
        // Wide title spanning both columns (more overlap on the right).
        let mut boxes = grid();
        boxes.push([120, 300, 290, 350]); // 4: spans, mostly right
        let ord = detect_reading_order(&boxes, ReadingMode::R2L);
        // Right column block (1,3,4) must precede left column block (0,2).
        let pos = |id: usize| ord.iter().position(|&x| x == id).unwrap();
        assert!(pos(1) < pos(0) && pos(3) < pos(0) && pos(4) < pos(0));
        assert!(pos(4) > pos(3)); // below BR inside right column
    }

    #[test]
    fn single_and_empty() {
        assert_eq!(
            detect_reading_order(&[], ReadingMode::R2L),
            Vec::<usize>::new()
        );
        assert_eq!(
            detect_reading_order(&[[0, 0, 10, 10]], ReadingMode::L2R),
            vec![0]
        );
    }
}
