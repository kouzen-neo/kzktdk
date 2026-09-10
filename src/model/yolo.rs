use anyhow::{Result, bail};
use image::{DynamicImage, GenericImageView, Rgb, RgbImage, imageops::FilterType};
use ndarray::Array4;
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;

pub const YOLO_INPUT_SIZE: u32 = 640;

pub const YOLO_PREDICTION_STAGES: [(f32, f32); 3] = [(0.28, 0.45), (0.18, 0.55), (0.10, 0.65)];

pub const MERGE_IOU_THRESHOLD: f32 = 0.50;
pub const MERGE_COVER_SMALL_THRESHOLD: f32 = 0.85;

pub const MIN_BOX_DIMENSION: f32 = 12.0;
pub const NONSENSE_AREA_RATIO: f32 = 0.035;
pub const NONSENSE_ASPECT_RATIO: f32 = 2.8;
pub const MAX_PAGE_COVERAGE_RATIO: f64 = 0.80;
pub const FALSE_GIANT_AREA_MULTIPLIER: f64 = 6.0;
pub const FALSE_GIANT_INTERSECTION_RATIO: f64 = 0.80;

pub const DARK_BG_LUMINANCE_THRESHOLD: f64 = 128.0;
pub const YOLO_GRID_640: usize = 8400;
pub const YOLO_GRID_SMALL: usize = 840;

#[derive(Debug, Clone)]
pub struct Detection {
    pub x1: u32,
    pub y1: u32,
    pub x2: u32,
    pub y2: u32,
    pub conf: f32,
}

impl Detection {
    pub fn width(&self) -> u32 {
        self.x2.saturating_sub(self.x1)
    }

    pub fn height(&self) -> u32 {
        self.y2.saturating_sub(self.y1)
    }

    pub fn area(&self) -> u64 {
        self.width() as u64 * self.height() as u64
    }

    pub fn iou(&self, other: &Detection) -> f32 {
        let ix1 = self.x1.max(other.x1);
        let iy1 = self.y1.max(other.y1);
        let ix2 = self.x2.min(other.x2);
        let iy2 = self.y2.min(other.y2);

        if ix2 <= ix1 || iy2 <= iy1 {
            return 0.0;
        }

        let inter_area = ((ix2 - ix1) as u64 * (iy2 - iy1) as u64) as f32;
        let union_area = (self.area() + other.area()) as f32 - inter_area;

        if union_area <= 0.0 {
            0.0
        } else {
            inter_area / union_area
        }
    }

    pub fn intersection_area(&self, other: &Detection) -> u64 {
        let ix1 = self.x1.max(other.x1);
        let iy1 = self.y1.max(other.y1);
        let ix2 = self.x2.min(other.x2);
        let iy2 = self.y2.min(other.y2);

        if ix2 <= ix1 || iy2 <= iy1 {
            0
        } else {
            (ix2 - ix1) as u64 * (iy2 - iy1) as u64
        }
    }

    /// Whether two boxes overlap enough to be merged (IoU >= 0.50 or coverSmall >= 0.85).
    /// Prevents distinct adjacent / double bubbles from being merged into one box.
    pub fn should_merge(&self, other: &Detection) -> bool {
        let a1 = self.area();
        let a2 = other.area();
        if a1 == 0 || a2 == 0 {
            return false;
        }
        let inter = self.intersection_area(other);
        if inter == 0 {
            return false;
        }
        let iou = inter as f32 / (a1 + a2 - inter) as f32;
        let cover_small = inter as f32 / (a1.min(a2) as f32);
        iou >= MERGE_IOU_THRESHOLD || cover_small >= MERGE_COVER_SMALL_THRESHOLD
    }

    /// Whether a box is too wide, flat, or thin to be a speech bubble (buang_kotak_ngawur).
    pub fn is_nonsense_box(&self, orig_w: u32, orig_h: u32) -> bool {
        let w = self.width() as f32;
        let h = self.height() as f32;
        if w < MIN_BOX_DIMENSION || h < MIN_BOX_DIMENSION {
            return true;
        }
        let ratio = w / h;
        let total_area = (orig_w as f32 * orig_h as f32).max(1.0);
        let area_ratio = (w * h) / total_area;
        let is_too_wide = ratio >= 3.2 && w >= orig_w as f32 * 0.35;
        let is_too_flat = w >= orig_w as f32 * 0.50 && h <= orig_h as f32 * 0.16;
        let is_too_thin = area_ratio >= NONSENSE_AREA_RATIO && ratio >= NONSENSE_ASPECT_RATIO;
        let spans_whole_page = self.area() >= (total_area as f64 * MAX_PAGE_COVERAGE_RATIO) as u64;

        is_too_wide || is_too_flat || is_too_thin || spans_whole_page
    }

    pub fn is_dark_bg(&self, img: &RgbImage) -> bool {
        let (w, h) = img.dimensions();
        let x1 = self.x1.min(w.saturating_sub(1));
        let y1 = self.y1.min(h.saturating_sub(1));
        let x2 = self.x2.min(w);
        let y2 = self.y2.min(h);

        let bw = x2.saturating_sub(x1);
        let bh = y2.saturating_sub(y1);
        if bw == 0 || bh == 0 {
            return false;
        }

        let step = (bw.min(bh) / 25).max(1);
        let mut total_lum = 0.0f64;
        let mut count = 0u64;

        let mut y = y1;
        while y < y2 {
            let mut x = x1;
            while x < x2 {
                let p = img.get_pixel(x, y);
                total_lum += 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64;
                count += 1;
                x += step;
            }
            y += step;
        }

        if count == 0 {
            return false;
        }

        (total_lum / count as f64) < DARK_BG_LUMINANCE_THRESHOLD
    }
}

pub struct PreparedInput {
    pub tensor: Array4<f32>,
    pub scale: f32,
    pub dw: f32,
    pub dh: f32,
}

pub struct YoloModel {
    session: Session,
}

impl YoloModel {
    pub fn new<P: AsRef<Path>>(model_path: P) -> Result<Self> {
        let session = Session::builder()?.commit_from_file(model_path)?;
        Ok(Self { session })
    }

    /// Preprocesses image into 640x640 letterboxed Float tensor (1, 3, 640, 640).
    pub fn prepare_input(img: &DynamicImage) -> PreparedInput {
        let (orig_w, orig_h) = img.dimensions();
        let target = YOLO_INPUT_SIZE as f32;

        let scale = (target / orig_h as f32).min(target / orig_w as f32);
        let new_w = (orig_w as f32 * scale).round() as u32;
        let new_h = (orig_h as f32 * scale).round() as u32;

        let dw = ((target - new_w as f32) / 2.0).floor();
        let dh = ((target - new_h as f32) / 2.0).floor();

        let resized = img.resize_exact(new_w, new_h, FilterType::Triangle);
        let mut padded =
            RgbImage::from_pixel(YOLO_INPUT_SIZE, YOLO_INPUT_SIZE, Rgb([114, 114, 114]));

        image::imageops::overlay(&mut padded, &resized.to_rgb8(), dw as i64, dh as i64);

        let mut tensor =
            Array4::<f32>::zeros((1, 3, YOLO_INPUT_SIZE as usize, YOLO_INPUT_SIZE as usize));
        for y in 0..YOLO_INPUT_SIZE {
            for x in 0..YOLO_INPUT_SIZE {
                let p = padded.get_pixel(x, y);
                tensor[[0, 0, y as usize, x as usize]] = p[0] as f32 / 255.0;
                tensor[[0, 1, y as usize, x as usize]] = p[1] as f32 / 255.0;
                tensor[[0, 2, y as usize, x as usize]] = p[2] as f32 / 255.0;
            }
        }

        PreparedInput {
            tensor,
            scale,
            dw,
            dh,
        }
    }

    /// Predicts detections using a single (conf, iou) threshold pair.
    pub fn predict(
        &mut self,
        orig_w: u32,
        orig_h: u32,
        conf_threshold: f32,
        iou_threshold: f32,
        prepared: &PreparedInput,
    ) -> Result<Vec<Detection>> {
        let input_tensor = Tensor::from_array(prepared.tensor.clone())?;
        let outputs = self.session.run(ort::inputs![input_tensor])?;

        let (_, output_tensor) = outputs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No output tensor returned"))?;

        let (_shape, raw_data) = output_tensor.try_extract_tensor::<f32>()?;
        let buf_size = raw_data.len();

        let (grid, channels) = if buf_size % YOLO_GRID_640 == 0 {
            (YOLO_GRID_640, buf_size / YOLO_GRID_640)
        } else if buf_size % YOLO_GRID_SMALL == 0 {
            (YOLO_GRID_SMALL, buf_size / YOLO_GRID_SMALL)
        } else {
            bail!("Unexpected output shape with {} floats", buf_size);
        };

        // Detect transposed vs direct layout
        let sample_direct = if buf_size > 4 * grid {
            raw_data[4 * grid]
        } else {
            -1.0
        };
        let sample_transposed = if buf_size > 4 { raw_data[4] } else { -1.0 };

        let transposed =
            (0.01..=1.0).contains(&sample_transposed) && !(0.01..=1.0).contains(&sample_direct);

        let mut raw_boxes = Vec::new();
        let scale = prepared.scale;
        let dw = prepared.dw;
        let dh = prepared.dh;

        for g in 0..grid {
            let (conf, xc, yc, bw, bh) = if transposed {
                (
                    raw_data[g * channels + 4],
                    raw_data[g * channels],
                    raw_data[g * channels + 1],
                    raw_data[g * channels + 2],
                    raw_data[g * channels + 3],
                )
            } else {
                (
                    raw_data[4 * grid + g],
                    raw_data[g],
                    raw_data[grid + g],
                    raw_data[2 * grid + g],
                    raw_data[3 * grid + g],
                )
            };

            if conf < conf_threshold {
                continue;
            }

            let x1 = ((xc - bw / 2.0 - dw) / scale).clamp(0.0, orig_w as f32);
            let y1 = ((yc - bh / 2.0 - dh) / scale).clamp(0.0, orig_h as f32);
            let x2 = ((xc + bw / 2.0 - dw) / scale).clamp(0.0, orig_w as f32);
            let y2 = ((yc + bh / 2.0 - dh) / scale).clamp(0.0, orig_h as f32);

            if x2 > x1 && y2 > y1 {
                raw_boxes.push(Detection {
                    x1: x1.round() as u32,
                    y1: y1.round() as u32,
                    x2: x2.round() as u32,
                    y2: y2.round() as u32,
                    conf,
                });
            }
        }

        Ok(Self::nms(raw_boxes, iou_threshold))
    }

    /// Full 3-stage YOLO cascade + box filtering (matches KZKT PagePreparer).
    pub fn detect_bubbles(&mut self, img: &DynamicImage) -> Result<Vec<Detection>> {
        let (orig_w, orig_h) = img.dimensions();
        let prepared = Self::prepare_input(img);
        let mut all_detections = Vec::new();

        for &(conf, iou) in &YOLO_PREDICTION_STAGES {
            let stage_dets = self.predict(orig_w, orig_h, conf, iou, &prepared)?;
            all_detections.extend(stage_dets);
        }

        // Post-filtering cascade:
        let filtered = Self::nms(all_detections, 0.45);
        let filtered = Self::remove_false_giants(filtered, orig_w, orig_h);
        let filtered = Self::remove_nonsense(filtered, orig_w, orig_h);
        let filtered = Self::merge_overlapping(filtered);

        Ok(filtered)
    }

    pub fn nms(mut boxes: Vec<Detection>, iou_threshold: f32) -> Vec<Detection> {
        boxes.sort_by(|a, b| {
            b.conf
                .partial_cmp(&a.conf)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut keep = Vec::new();

        for b in boxes {
            let overlaps = keep.iter().any(|k: &Detection| k.iou(&b) > iou_threshold);
            if !overlaps {
                keep.push(b);
            }
        }

        keep
    }

    /// Removes giant candidate boxes that swallow real bubbles (area > 6.0 * child and intersection >= 0.8 * child).
    pub fn remove_false_giants(boxes: Vec<Detection>, orig_w: u32, orig_h: u32) -> Vec<Detection> {
        if boxes.is_empty() {
            return boxes;
        }
        let total_area = orig_w as u64 * orig_h as u64;
        let mut with_area: Vec<Detection> = boxes
            .into_iter()
            .filter(|b| b.area() < (total_area as f64 * MAX_PAGE_COVERAGE_RATIO) as u64)
            .collect();
        with_area.sort_by_key(|a| std::cmp::Reverse(a.area()));

        let len = with_area.len();
        let mut keep = vec![true; len];

        for i in 0..len {
            if !keep[i] {
                continue;
            }
            let area_i = with_area[i].area() as f64;
            for j in (i + 1)..len {
                if !keep[j] {
                    continue;
                }
                let area_j = with_area[j].area() as f64;
                if area_i > FALSE_GIANT_AREA_MULTIPLIER * area_j {
                    let inter = with_area[i].intersection_area(&with_area[j]) as f64;
                    if inter >= FALSE_GIANT_INTERSECTION_RATIO * area_j {
                        keep[i] = false;
                        break;
                    }
                }
            }
        }

        with_area
            .into_iter()
            .enumerate()
            .filter(|(idx, _)| keep[*idx])
            .map(|(_, b)| b)
            .collect()
    }

    /// Drops boxes that are too wide, flat, thin, or degenerate (buang_kotak_ngawur).
    pub fn remove_nonsense(boxes: Vec<Detection>, orig_w: u32, orig_h: u32) -> Vec<Detection> {
        boxes
            .into_iter()
            .filter(|b| !b.is_nonsense_box(orig_w, orig_h))
            .collect()
    }

    /// Iteratively merges overlapping boxes based on BoxGeometry.shouldMerge (IoU >= 0.50 or coverSmall >= 0.85).
    pub fn merge_overlapping(boxes: Vec<Detection>) -> Vec<Detection> {
        if boxes.is_empty() {
            return boxes;
        }
        let mut result = boxes;
        result.sort_by_key(|b| b.x1);

        let mut changed = true;
        while changed {
            changed = false;
            let mut new_boxes = Vec::new();
            let mut used = vec![false; result.len()];

            for i in 0..result.len() {
                if used[i] {
                    continue;
                }
                let mut curr = result[i].clone();

                for j in (i + 1)..result.len() {
                    if used[j] {
                        continue;
                    }
                    if result[j].x1 > curr.x2 {
                        break;
                    }
                    if curr.should_merge(&result[j]) {
                        curr.x1 = curr.x1.min(result[j].x1);
                        curr.y1 = curr.y1.min(result[j].y1);
                        curr.x2 = curr.x2.max(result[j].x2);
                        curr.y2 = curr.y2.max(result[j].y2);
                        curr.conf = curr.conf.max(result[j].conf);
                        used[j] = true;
                        changed = true;
                    }
                }
                new_boxes.push(curr);
                used[i] = true;
            }
            result = new_boxes;
        }

        result.sort_by_key(|b| (b.y1 as u64) * 10000 + b.x1 as u64);
        result
    }
}

#[cfg(test)]
mod send_tests {
    use super::*;

    fn assert_send<T: Send>() {}

    /// Compile-time proof that the ONNX session can be shared across threads
    /// (required for the YOLO model pool in batch translate and for Tauri
    /// managed state). Fails to compile if `ort::Session` ever loses `Send`.
    #[test]
    fn ort_session_and_yolo_model_are_send() {
        assert_send::<ort::session::Session>();
        assert_send::<YoloModel>();
    }
}
