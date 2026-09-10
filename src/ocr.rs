use image::{DynamicImage, RgbImage, imageops::FilterType};
use ndarray::Array4;
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::collections::HashMap;

// ---------- Session cache ----------

static SESSION_CACHE: OnceLock<Mutex<HashMap<String, Arc<Mutex<Session>>>>> = OnceLock::new();

fn get_or_load_session(path: &Path) -> anyhow::Result<Arc<Mutex<Session>>> {
    let cache = SESSION_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key = path.to_string_lossy().to_string();
    
    let mut lock = cache.lock().map_err(|e| anyhow::anyhow!("cache lock poisoned: {}", e))?;
    if let Some(sess) = lock.get(&key) {
        return Ok(Arc::clone(sess));
    }
    
    let sess = Session::builder()?.commit_from_file(path)?;
    let wrapped = Arc::new(Mutex::new(sess));
    lock.insert(key, Arc::clone(&wrapped));
    Ok(wrapped)
}

// ---------- TextRegion ----------

#[derive(Debug, Clone)]
pub struct TextRegion {
    pub bbox: [u32; 4],
    pub text: String,
}

// ---------- OcrScript ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OcrScript {
    Japanese,
    English,
    Korean,
    Chinese,
    ChineseTraditional,
    Auto,
}

impl OcrScript {
    pub fn from_key(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "jp" | "japanese" | "ja" => Self::Japanese,
            "en" | "english" => Self::English,
            "kr" | "korean" | "ko" => Self::Korean,
            "cn" | "chinese" | "zh" => Self::Chinese,
            "cht" | "zh-tw" | "zh-hk" => Self::ChineseTraditional,
            "auto" => Self::Auto,
            _ => Self::Japanese,
        }
    }
}

// ---------- OcrEngine trait ----------

/// Abstraction for pluggable OCR engines.
pub trait OcrEngine: Send + Sync {
    fn recognize(&self, _image: &RgbImage, _bbox: [u32; 4]) -> Option<String> {
        None
    }
    fn recognize_regions(&self, _full: &RgbImage, _exclude: &[[u32; 4]]) -> Vec<TextRegion> {
        Vec::new()
    }
    fn name(&self) -> &'static str {
        "none"
    }
}

// ---------- rec support ----------
//
// Text *recognition* (reading characters) is NOT implemented for the
// rapid/manga stub engines: they can detect boxes but must never guess
// text. `tesseract` delegates to the real external binary (when installed).
// Callers that explicitly need rec (`--translate-free-text`, `--mode ocr`)
// must fail fast via `ensure_rec_available` instead of translating lies.

/// True when `name` selects a KNOWN engine without real text recognition
/// (manga stub only now; rapid has rec). Unknown names fall back to noop elsewhere.
pub fn engine_rec_stub(name: &str) -> bool {
    matches!(name.to_lowercase().as_str(), "manga")
}

/// Fail fast when rec is explicitly required but the engine is a stub or script unsupported.
pub fn ensure_rec_available(ocr: &str, script: OcrScript) -> anyhow::Result<()> {
    if engine_rec_stub(ocr) {
        anyhow::bail!(
            "OCR text recognition (rec) is not implemented for engine '{ocr}': \
             stub engine cannot read text, so the requested operation cannot run. \
             Use --ocr tesseract or --ocr rapid instead."
        );
    }
    if ocr.to_lowercase() == "rapid" && script == OcrScript::ChineseTraditional {
        anyhow::bail!(
            "OCR script 'cht' (Traditional Chinese) is not yet supported: \
             no ONNX model available for download. Use 'cn' (Simplified) instead, \
             or --ocr tesseract with chi_tra traineddata if installed."
        );
    }
    Ok(())
}

fn warn_rec_stub_once(engine: &'static str) {
    static WARNED_RAPID: std::sync::Once = std::sync::Once::new();
    static WARNED_MANGA: std::sync::Once = std::sync::Once::new();
    let once = match engine {
        "rapid" => &WARNED_RAPID,
        _ => &WARNED_MANGA,
    };
    once.call_once(|| {
        eprintln!(
            "[OCR] engine '{engine}': text recognition (rec) is NOT implemented — \
             returning no text (never guessing). Detection boxes still work."
        );
    });
}

// ---------- Noop ----------

pub struct NoopOcr;
impl OcrEngine for NoopOcr {
    fn recognize(&self, _image: &RgbImage, _bbox: [u32; 4]) -> Option<String> {
        None
    }
    fn recognize_regions(&self, _full: &RgbImage, _exclude: &[[u32; 4]]) -> Vec<TextRegion> {
        Vec::new()
    }
    fn name(&self) -> &'static str {
        "none"
    }
}

// ---------- Stub engines (rapid/manga) ----------
// Detection may work when models are cached, but text recognition (rec) is
// NOT implemented: they return no text and fail fast when rec is required
// (see ensure_rec_available). Tesseract below is real (external binary).

pub struct RapidOcr {
    pub model_path: Option<PathBuf>,
    pub det_path: Option<PathBuf>,
    pub rec_path: Option<PathBuf>,
    pub dict_path: Option<PathBuf>,
    pub script: OcrScript,
}
impl OcrEngine for RapidOcr {
    fn recognize(&self, image: &RgbImage, bbox: [u32; 4]) -> Option<String> {
        // if real model present, try ort rec; never guess
        if let (Some(rec), Some(dict)) = (&self.rec_path, &self.dict_path) {
            if rec.exists() && dict.exists() {
                if let Some(s) = rapid_recognize_crop(image, bbox, rec, dict) {
                    return Some(s);
                }
            }
        }
        warn_rec_stub_once("rapid");
        None
    }
    fn recognize_regions(&self, full: &RgbImage, _exclude: &[[u32; 4]]) -> Vec<TextRegion> {
        if full.width() < 20 || full.height() < 20 {
            return Vec::new();
        }
        if let (Some(det), Some(rec), Some(dict)) =
            (&self.det_path, &self.rec_path, &self.dict_path)
        {
            if det.exists() && rec.exists() && dict.exists() {
                let regs = rapid_detect_regions(full, det, rec, dict, self.script);
                if !regs.is_empty() {
                    return regs;
                }
            }
        }
        // No models (or empty detection): honest empty, never fake boxes.
        Vec::new()
    }
    fn name(&self) -> &'static str {
        "rapid"
    }
}

fn rapid_detect_regions(full: &RgbImage, det: &Path, rec: &Path, dict: &Path, script: OcrScript) -> Vec<TextRegion> {
    match rapid_detect_ort(full, det, rec, dict, script) {
        Ok(v) => v,
        Err(e) => {
            eprintln!(
                "[rapid] det failed: {e} — no regions"
            );
            Vec::new()
        }
    }
}
fn rapid_detect_ort(
    full: &RgbImage,
    det: &Path,
    rec: &Path,
    dict: &Path,
    script: OcrScript,
) -> anyhow::Result<Vec<TextRegion>> {
    let (orig_w, orig_h) = (full.width(), full.height());
    // letterbox to 640 like YOLO
    let dyn_img = DynamicImage::ImageRgb8(full.clone());
    let target = 640f32;
    let scale = (target / orig_h as f32).min(target / orig_w as f32);
    let new_w = (orig_w as f32 * scale).round() as u32;
    let new_h = (orig_h as f32 * scale).round() as u32;
    let dw = ((target - new_w as f32) / 2.0).floor() as i64;
    let dh = ((target - new_h as f32) / 2.0).floor() as i64;
    let resized = dyn_img.resize_exact(new_w, new_h, FilterType::Triangle);
    let mut padded = RgbImage::from_pixel(640, 640, image::Rgb([114, 114, 114]));
    image::imageops::overlay(&mut padded, &resized.to_rgb8(), dw, dh);
    let mut tensor = Array4::<f32>::zeros((1, 3, 640, 640));
    for y in 0..640 {
        for x in 0..640 {
            let p = padded.get_pixel(x, y);
            // Paddle det uses mean 0.485/0.456/0.406 std 0.229/0.224/0.225 but 0-1 works for now
            tensor[[0, 0, y as usize, x as usize]] = p[0] as f32 / 255.0;
            tensor[[0, 1, y as usize, x as usize]] = p[1] as f32 / 255.0;
            tensor[[0, 2, y as usize, x as usize]] = p[2] as f32 / 255.0;
        }
    }
    let sess_arc = get_or_load_session(det)?;
    let input = Tensor::from_array(tensor)?;
    
    // Extract tensor data inside lock to avoid lifetime issues
    let (shape_vec, data_vec): (Vec<i64>, Vec<f32>) = {
        let mut sess = sess_arc.lock().map_err(|e| anyhow::anyhow!("session lock poisoned: {}", e))?;
        let outputs = sess.run(ort::inputs![input])?;
        let (_, val) = outputs
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("no output"))?;
        let (shape, data) = val.try_extract_tensor::<f32>()?;
        (shape.to_vec(), data.to_vec())
    };
    
    let shape = &shape_vec;
    let data = &data_vec;
    // shape e.g. [1,1,640,640] or [1,640,640]
    let h = if shape.len() >= 4 {
        shape[2] as usize
    } else if shape.len() == 3 {
        shape[1] as usize
    } else {
        640
    };
    let w = if shape.len() >= 4 {
        shape[3] as usize
    } else if shape.len() == 3 {
        shape[2] as usize
    } else {
        640
    };
    // find boxes via threshold >0.3
    let thresh = 0.3f32;
    let mut mask = vec![vec![false; w]; h];
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if idx < data.len() && data[idx] > thresh {
                mask[y][x] = true;
            }
        }
    }
    // connected components BFS 4-dir
    let mut visited = vec![vec![false; w]; h];
    let mut boxes: Vec<[u32; 4]> = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if !mask[y][x] || visited[y][x] {
                continue;
            }
            let mut q = vec![(y, x)];
            visited[y][x] = true;
            let (mut minx, mut miny, mut maxx, mut maxy) = (x, y, x, y);
            let mut qi = 0;
            while qi < q.len() {
                let (cy, cx) = q[qi];
                qi += 1;
                for (ny, nx) in [
                    (cy.wrapping_sub(1), cx),
                    (cy + 1, cx),
                    (cy, cx.wrapping_sub(1)),
                    (cy, cx + 1),
                ] {
                    if ny >= h || nx >= w {
                        continue;
                    }
                    if !mask[ny][nx] || visited[ny][nx] {
                        continue;
                    }
                    visited[ny][nx] = true;
                    q.push((ny, nx));
                    minx = minx.min(nx);
                    miny = miny.min(ny);
                    maxx = maxx.max(nx);
                    maxy = maxy.max(ny);
                }
            }
            let bw = maxx - minx + 1;
            let bh = maxy - miny + 1;
            if bw < 8 || bh < 8 {
                continue;
            }
            // map back to orig via scale/dw/dh
            let x1 = ((minx as f32 - dw as f32) / scale)
                .clamp(0.0, orig_w as f32)
                .round() as u32;
            let y1 = ((miny as f32 - dh as f32) / scale)
                .clamp(0.0, orig_h as f32)
                .round() as u32;
            let x2 = ((maxx as f32 - dw as f32) / scale)
                .clamp(0.0, orig_w as f32)
                .round() as u32;
            let y2 = ((maxy as f32 - dh as f32) / scale)
                .clamp(0.0, orig_h as f32)
                .round() as u32;
            if x2 > x1 && y2 > y1 {
                boxes.push([x1, y1, x2, y2]);
            }
        }
    }
    
    // Trial-decode for Auto script
    let (final_rec, final_dict) = if script == OcrScript::Auto {
        if boxes.is_empty() {
            return Ok(Vec::new());
        }
        
        // Select sample boxes: largest 2-3 boxes by area
        let mut boxes_with_area: Vec<([u32; 4], u32)> = boxes
            .iter()
            .map(|b| {
                let area = (b[2] - b[0]) * (b[3] - b[1]);
                (*b, area)
            })
            .collect();
        boxes_with_area.sort_by(|a, b| b.1.cmp(&a.1));
        
        let sample_boxes: Vec<[u32; 4]> = boxes_with_area
            .iter()
            .filter(|(_, area)| *area > 100)
            .take(3)
            .map(|(b, _)| *b)
            .collect();
        
        if sample_boxes.is_empty() {
            return Ok(Vec::new());
        }
        
        let cache_dir = cache_dir_for("rapid");
        let detected_script = trial_decode_language(full, &sample_boxes, &cache_dir);
        
        match detected_script {
            Some(s) => {
                if let Some((rec_url, dict_url)) = crate::config::rapid_model_urls(s) {
                    let rec_name = rec_url.split('/').last().unwrap_or("rec.onnx");
                    let dict_name = dict_url.split('/').last().unwrap_or("dict.txt");
                    (cache_dir.join(rec_name), cache_dir.join(dict_name))
                } else {
                    return Ok(Vec::new());
                }
            }
            None => {
                return Ok(Vec::new());
            }
        }
    } else {
        (rec.to_path_buf(), dict.to_path_buf())
    };
    
    // for each box try rec to get text; skip boxes we cannot read
    // (never emit placeholder text)
    let mut regs = Vec::new();
    for b in boxes {
        if let Some(txt) = rapid_recognize_crop(full, b, &final_rec, &final_dict) {
            regs.push(TextRegion { bbox: b, text: txt });
        }
    }
    Ok(regs)
}
fn rapid_recognize_crop_with_confidence(img: &RgbImage, bbox: [u32; 4], rec: &Path, dict: &Path) -> Option<(String, f32)> {
    let (x1, y1, x2, y2) = (bbox[0], bbox[1], bbox[2], bbox[3]);
    if x2 <= x1 || y2 <= y1 {
        return None;
    }
    let w = x2 - x1;
    let h = y2 - y1;
    if w < 4 || h < 4 {
        return None;
    }
    
    // Crop
    let crop = image::imageops::crop_imm(img, x1, y1, w, h).to_image();
    
    // Check if vertical (tategaki): rotate 90 degrees
    let crop = if h > w {
        image::imageops::rotate90(&crop)
    } else {
        crop
    };
    
    // Resize to height 32, keep aspect ratio, pad right
    let (cw, ch) = (crop.width(), crop.height());
    let target_h = 32u32;
    let scale = target_h as f32 / ch as f32;
    let new_w = (cw as f32 * scale).round() as u32;
    let resized = image::imageops::resize(&crop, new_w, target_h, FilterType::Triangle);
    
    // Pad to at least 32 width (common models accept variable width, but normalize)
    let padded_w = new_w.max(32);
    let mut padded = RgbImage::from_pixel(padded_w, target_h, image::Rgb([255, 255, 255]));
    image::imageops::overlay(&mut padded, &resized, 0, 0);
    
    // Normalize to [0,1] with mean/std 0.5 (simple normalization for now)
    let mut tensor = ndarray::Array4::<f32>::zeros((1, 3, target_h as usize, padded_w as usize));
    for y in 0..target_h {
        for x in 0..padded_w {
            let p = padded.get_pixel(x, y);
            for c in 0..3 {
                tensor[[0, c, y as usize, x as usize]] = (p[c] as f32 / 255.0 - 0.5) / 0.5;
            }
        }
    }
    
    // Load dict
    let dict_content = std::fs::read_to_string(dict).ok()?;
    let dict_chars: Vec<&str> = dict_content.lines().collect();
    let num_classes = dict_chars.len() + 1; // +1 for blank
    
    // ORT session with cache - extract data inside lock
    let (shape_vec, data_vec): (Vec<i64>, Vec<f32>) = {
        let sess_arc = get_or_load_session(rec).ok()?;
        let input = Tensor::from_array(tensor).ok()?;
        let mut sess = sess_arc.lock().ok()?;
        let outputs = sess.run(ort::inputs![input]).ok()?;
        let (_, val) = outputs.into_iter().next()?;
        let (shape, data) = val.try_extract_tensor::<f32>().ok()?;
        (shape.to_vec(), data.to_vec())
    };
    
    let shape = &shape_vec;
    let data = &data_vec;
    
    // Shape typically [T, 1, num_classes] or [1, T, num_classes]
    let (t_steps, classes) = if shape.len() == 3 {
        if shape[1] == 1 {
            (shape[0] as usize, shape[2] as usize)
        } else {
            (shape[1] as usize, shape[2] as usize)
        }
    } else if shape.len() == 2 {
        (shape[0] as usize, shape[1] as usize)
    } else {
        return None;
    };
    
    // Assert compatibility
    if classes != num_classes {
        eprintln!(
            "[rapid] dict/model mismatch: dict {} + blank = {}, model output = {}",
            dict_chars.len(), num_classes, classes
        );
        return None;
    }
    
    // CTC decode: argmax + collapse
    let mut indices = Vec::new();
    let mut confidences = Vec::new();
    for t in 0..t_steps {
        let offset = if shape.len() == 3 && shape[1] == 1 {
            t * classes
        } else if shape.len() == 3 {
            t * classes
        } else {
            t * classes
        };
        
        let mut max_idx = 0;
        let mut max_val = data[offset];
        for c in 1..classes {
            let v = data[offset + c];
            if v > max_val {
                max_val = v;
                max_idx = c;
            }
        }
        indices.push(max_idx);
        confidences.push(max_val);
    }
    
    // Collapse: remove blank (0) and consecutive duplicates
    let mut result = String::new();
    let mut prev = None;
    let mut valid_confidences = Vec::new();
    for (i, &idx) in indices.iter().enumerate() {
        if idx == 0 {
            prev = None;
            continue;
        }
        if Some(idx) == prev {
            continue;
        }
        prev = Some(idx);
        if idx > 0 && idx <= dict_chars.len() {
            result.push_str(dict_chars[idx - 1]);
            valid_confidences.push(confidences[i]);
        }
    }
    
    if result.is_empty() {
        None
    } else {
        let avg_conf = valid_confidences.iter().sum::<f32>() / valid_confidences.len() as f32;
        Some((result, avg_conf))
    }
}

fn rapid_recognize_crop(img: &RgbImage, bbox: [u32; 4], rec: &Path, dict: &Path) -> Option<String> {
    rapid_recognize_crop_with_confidence(img, bbox, rec, dict).map(|(text, _conf)| text)
}

/// Trial-decode 2 sample crops with all available models, return best script.
/// Returns None if all confidences below threshold (unreadable).
fn trial_decode_language(
    img: &RgbImage,
    sample_boxes: &[[u32; 4]],
    cache_dir: &Path,
) -> Option<OcrScript> {
    let scripts = [
        OcrScript::English,
        OcrScript::Japanese,
        OcrScript::Korean,
        OcrScript::Chinese,
    ];
    
    let mut best_script = None;
    let mut best_conf = 0.0;
    
    for script in scripts {
        let Some((rec_url, dict_url)) = crate::config::rapid_model_urls(script) else { continue };
        let rec_name = rec_url.split('/').last().unwrap_or("rec.onnx");
        let dict_name = dict_url.split('/').last().unwrap_or("dict.txt");
        let rec = cache_dir.join(rec_name);
        let dict = cache_dir.join(dict_name);
        
        if !rec.exists() || !dict.exists() {
            continue;
        }
        
        let mut total_conf = 0.0;
        let mut count = 0;
        
        for bbox in sample_boxes.iter().take(2) {
            if let Some((_text, conf)) = rapid_recognize_crop_with_confidence(img, *bbox, &rec, &dict) {
                total_conf += conf;
                count += 1;
            }
        }
        
        if count > 0 {
            let avg_conf = total_conf / count as f32;
            if avg_conf > best_conf {
                best_conf = avg_conf;
                best_script = Some(script);
            }
        }
    }
    
    if best_conf < 0.3 {
        eprintln!("[rapid-auto] all models low confidence ({:.2}), unreadable", best_conf);
        return None;
    }
    
    if let Some(s) = best_script {
        eprintln!("[rapid-auto] detected {:?} (conf {:.2})", s, best_conf);
    }
    best_script
}

pub struct MangaOcr {
    pub model_path: Option<PathBuf>,
}
impl OcrEngine for MangaOcr {
    fn recognize(&self, _image: &RgbImage, _bbox: [u32; 4]) -> Option<String> {
        warn_rec_stub_once("manga");
        None
    }
    fn recognize_regions(&self, _full: &RgbImage, _exclude: &[[u32; 4]]) -> Vec<TextRegion> {
        // Stub: no detection implementation — honest empty, never fake boxes.
        Vec::new()
    }
    fn name(&self) -> &'static str {
        "manga"
    }
}

pub struct TesseractOcr {
    pub script: OcrScript,
}
impl TesseractOcr {
    fn lang(&self) -> String {
        match self.script {
            OcrScript::Japanese => "jpn_vert+jpn".to_string(),
            OcrScript::English => "eng".to_string(),
            OcrScript::Korean => "kor+eng".to_string(),
            OcrScript::Chinese => "chi_sim+chi_tra+eng".to_string(),
            OcrScript::ChineseTraditional => "chi_tra+eng".to_string(),
            OcrScript::Auto => "jpn_vert+jpn+eng".to_string(),
        }
    }
}
impl OcrEngine for TesseractOcr {
    fn recognize(&self, image: &RgbImage, bbox: [u32; 4]) -> Option<String> {
        let (x1, y1, x2, y2) = (bbox[0], bbox[1], bbox[2], bbox[3]);
        if x2 <= x1 || y2 <= y1 {
            return None;
        }
        let w = x2 - x1;
        let h = y2 - y1;
        if w < 8 || h < 8 {
            return None;
        }
        // crop
        let mut crop = RgbImage::new(w, h);
        for cy in 0..h {
            for cx in 0..w {
                if x1 + cx < image.width() && y1 + cy < image.height() {
                    crop.put_pixel(cx, cy, *image.get_pixel(x1 + cx, y1 + cy));
                }
            }
        }
        tesseract_ocr_image(&crop, &self.lang())
    }
    fn recognize_regions(&self, full: &RgbImage, _exclude: &[[u32; 4]]) -> Vec<TextRegion> {
        if full.width() < 20 || full.height() < 20 {
            return Vec::new();
        }
        tesseract_tsv_regions(full, &self.lang())
    }
    fn name(&self) -> &'static str {
        "tesseract"
    }
}

fn tesseract_ocr_image(img: &RgbImage, lang: &str) -> Option<String> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("kzktdk_tess_{}.png", std::process::id()));
    if img.save(&path).is_err() {
        return None;
    }
    let out = std::process::Command::new("tesseract")
        .arg(&path)
        .arg("stdout")
        .arg("-l")
        .arg(lang)
        .arg("--psm")
        .arg("6")
        .arg("--oem")
        .arg("1")
        .output();
    let _ = std::fs::remove_file(&path);
    match out {
        Ok(o) if o.status.success() => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if s.is_empty() { None } else { Some(s) }
        }
        _ => None,
    }
}

fn tesseract_tsv_regions(img: &RgbImage, lang: &str) -> Vec<TextRegion> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("kzktdk_tsv_{}.png", std::process::id()));
    if img.save(&path).is_err() {
        return Vec::new();
    }
    let out = std::process::Command::new("tesseract")
        .arg(&path)
        .arg("stdout")
        .arg("-l")
        .arg(lang)
        .arg("--psm")
        .arg("6")
        .arg("--oem")
        .arg("1")
        .arg("tsv")
        .output();
    let _ = std::fs::remove_file(&path);
    let mut regs = Vec::new();
    if let Ok(o) = out {
        if o.status.success() {
            let txt = String::from_utf8_lossy(&o.stdout);
            for line in txt.lines().skip(1) {
                let cols: Vec<&str> = line.split('\t').collect();
                if cols.len() < 12 {
                    continue;
                }
                let conf: i32 = cols[10].parse().unwrap_or(-1);
                if conf < 30 {
                    continue;
                }
                let text = cols[11].trim();
                if text.is_empty() {
                    continue;
                }
                let x: u32 = cols[6].parse().unwrap_or(0);
                let y: u32 = cols[7].parse().unwrap_or(0);
                let w: u32 = cols[8].parse().unwrap_or(0);
                let h: u32 = cols[9].parse().unwrap_or(0);
                if w < 12 || h < 8 {
                    continue;
                }
                regs.push(TextRegion {
                    bbox: [x, y, x + w, y + h],
                    text: text.to_string(),
                });
            }
        }
    }
    // cluster nearby words into lines via merge handled in preparer; return raw
    regs
}

// ---------- Engine factory + auto-download ----------

fn cache_dir_for(engine: &str) -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".cache/kzktdk/models")
            .join(engine)
    } else {
        PathBuf::from("/tmp/kzktdk/models").join(engine)
    }
}

fn download_file(url: &str, dst: &Path) -> bool {
    if dst.exists() {
        return true;
    }
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    eprintln!("[OCR] downloading {} -> {:?}", url, dst);
    // try curl first (handles HF redirects + large files)
    let out = std::process::Command::new("curl")
        .arg("-L")
        .arg("-f")
        .arg("-o")
        .arg(dst)
        .arg(url)
        .output();
    if let Ok(o) = out {
        if o.status.success() && dst.exists() {
            return true;
        }
        eprintln!("[OCR] curl failed: {}", String::from_utf8_lossy(&o.stderr));
    }
    // fallback reqwest via tokio if curl missing
    false
}

fn ensure_rapid_cached(custom_path: Option<&Path>, script: OcrScript) -> Option<(PathBuf, PathBuf, PathBuf)> {
    if let Some(p) = custom_path {
        if p.exists() {
            return Some((p.to_path_buf(), p.to_path_buf(), p.to_path_buf()));
        }
        eprintln!("[OCR] custom --ocr-model {:?} not found", p);
    }
    let dir = cache_dir_for("rapid");
    let _ = std::fs::create_dir_all(&dir);
    let det = dir.join("ch_PP-OCRv3_det_infer.onnx");
    
    // For auto, download all supported scripts
    let scripts_to_download = if script == OcrScript::Auto {
        vec![OcrScript::English, OcrScript::Japanese, OcrScript::Korean, OcrScript::Chinese]
    } else {
        vec![script]
    };
    
    if !det.exists() {
        let det_url = crate::config::RAPIDOCR_DET_URL;
        let _ = download_file(det_url, &det);
    }
    
    for s in scripts_to_download {
        if let Some((rec_url, dict_url)) = crate::config::rapid_model_urls(s) {
            let rec_name = rec_url.split('/').last().unwrap_or("rec.onnx");
            let dict_name = dict_url.split('/').last().unwrap_or("dict.txt");
            let rec = dir.join(rec_name);
            let dict = dir.join(dict_name);
            
            if !rec.exists() {
                eprintln!("[OCR] downloading {:?} rec model...", s);
                let _ = download_file(rec_url, &rec);
            }
            if !dict.exists() {
                let _ = download_file(dict_url, &dict);
            }
        }
    }
    
    // Return paths for the requested script (or first available if auto)
    let target_script = if script == OcrScript::Auto {
        OcrScript::Japanese
    } else {
        script
    };
    
    if let Some((rec_url, dict_url)) = crate::config::rapid_model_urls(target_script) {
        let rec_name = rec_url.split('/').last().unwrap_or("rec.onnx");
        let dict_name = dict_url.split('/').last().unwrap_or("dict.txt");
        let rec = dir.join(rec_name);
        let dict = dir.join(dict_name);
        
        if det.exists() && rec.exists() && dict.exists() {
            return Some((det, rec, dict));
        }
    }
    
    None
}

fn ensure_model_cached(engine: &str, custom_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = custom_path {
        if p.exists() {
            return Some(p.to_path_buf());
        }
        eprintln!("[OCR] custom --ocr-model {:?} not found, fallback cache", p);
    }
    let dir = cache_dir_for(engine);
    if dir.exists() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension()
                    .map(|e| e.eq_ignore_ascii_case("onnx"))
                    .unwrap_or(false)
                {
                    return Some(p);
                }
                if p.is_dir() {
                    if let Ok(rd2) = std::fs::read_dir(&p) {
                        for e2 in rd2.flatten() {
                            let pp = e2.path();
                            if pp
                                .extension()
                                .map(|e| e.eq_ignore_ascii_case("onnx"))
                                .unwrap_or(false)
                            {
                                return Some(pp);
                            }
                        }
                    }
                }
            }
        }
    }
    match engine {
        "rapid" => {
            eprintln!(
                "[OCR] rapid model not cached at {:?} — expected det+rec .onnx (SWHL/RapidOCR). Run with --ocr-model <path> or place model.",
                dir
            );
        }
        "manga" => {
            eprintln!(
                "[OCR] manga model not cached at {:?} — expected onnx-community/manga-ocr-base-ONNX. Run with --ocr-model <path>.",
                dir
            );
        }
        _ => {}
    }
    let _ = std::fs::create_dir_all(&dir);
    None
}

pub fn create_ocr_engine(
    name: &str,
    ocr_model: Option<&Path>,
    script: OcrScript,
) -> Box<dyn OcrEngine> {
    match name.to_lowercase().as_str() {
        "none" => Box::new(NoopOcr),
        "rapid" => {
            let cached = ensure_rapid_cached(ocr_model, script);
            if let Some((det, rec, dict)) = cached {
                Box::new(RapidOcr {
                    model_path: Some(det.clone()),
                    det_path: Some(det),
                    rec_path: Some(rec),
                    dict_path: Some(dict),
                    script,
                })
            } else {
                let p = ensure_model_cached("rapid", ocr_model);
                Box::new(RapidOcr {
                    model_path: p,
                    det_path: None,
                    rec_path: None,
                    dict_path: None,
                    script,
                })
            }
        }
        "manga" => {
            let p = ensure_model_cached("manga", ocr_model);
            Box::new(MangaOcr { model_path: p })
        }
        "tesseract" => Box::new(TesseractOcr { script }),
        // legacy aliases
        "vision" | "local" => {
            eprintln!(
                "[OCR] engine '{}' deprecated, use rapid|manga|tesseract, fallback none",
                name
            );
            Box::new(NoopOcr)
        }
        other => {
            eprintln!("[OCR] unknown engine '{}', fallback none", other);
            Box::new(NoopOcr)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_engines_report_no_rec() {
        assert!(engine_rec_stub("manga"));
        assert!(engine_rec_stub("MANGA"));
        assert!(!engine_rec_stub("rapid"));
        assert!(!engine_rec_stub("tesseract"));
        assert!(!engine_rec_stub("none"));
        assert!(!engine_rec_stub("bogus"));
    }

    #[test]
    fn ensure_rec_available_fails_fast_for_stubs() {
        assert!(ensure_rec_available("manga", OcrScript::Japanese).is_err());
        assert!(ensure_rec_available("tesseract", OcrScript::Japanese).is_ok());
        assert!(ensure_rec_available("none", OcrScript::Japanese).is_ok());
        assert!(ensure_rec_available("rapid", OcrScript::Japanese).is_ok());
        assert!(ensure_rec_available("rapid", OcrScript::ChineseTraditional).is_err());
        let e = ensure_rec_available("rapid", OcrScript::ChineseTraditional).unwrap_err();
        assert!(e.to_string().contains("not yet supported"), "unexpected: {e}");
    }

    #[test]
    fn stub_engines_never_guess_text() {
        let img = RgbImage::from_pixel(64, 64, image::Rgb([255, 255, 255]));
        let rapid = RapidOcr {
            model_path: None,
            det_path: None,
            rec_path: None,
            dict_path: None,
            script: OcrScript::Japanese,
        };
        assert_eq!(rapid.recognize(&img, [5, 5, 50, 50]), None);
        assert!(rapid.recognize_regions(&img, &[]).is_empty());
        let manga = MangaOcr { model_path: None };
        assert_eq!(manga.recognize(&img, [5, 5, 50, 50]), None);
        assert!(manga.recognize_regions(&img, &[]).is_empty());
    }

    #[test]
    fn rapid_model_urls_cover_supported_scripts() {
        use crate::config::rapid_model_urls;
        assert!(rapid_model_urls(OcrScript::Japanese).is_some());
        assert!(rapid_model_urls(OcrScript::Korean).is_some());
        assert!(rapid_model_urls(OcrScript::English).is_some());
        assert!(rapid_model_urls(OcrScript::Chinese).is_some());
        assert!(rapid_model_urls(OcrScript::ChineseTraditional).is_none());
        assert!(rapid_model_urls(OcrScript::Auto).is_none());
        
        let (rec, dict) = rapid_model_urls(OcrScript::Japanese).unwrap();
        assert!(rec.contains("japan_rec_crnn.onnx"));
        assert!(dict.contains("japan_dict.txt"));
    }
}
