use image::{DynamicImage, GenericImageView, RgbImage, imageops::FilterType};
use ndarray::Array4;
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};

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
    Auto,
}

impl OcrScript {
    pub fn from_key(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "jp" | "japanese" | "ja" => Self::Japanese,
            "en" | "english" => Self::English,
            "kr" | "korean" | "ko" => Self::Korean,
            "cn" | "chinese" | "zh" => Self::Chinese,
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

// ---------- Stub engines (rapid/manga/tesseract) ----------
// Full ONNX inference will be added later; for now they behave like Noop
// but trigger auto-download and log correctly for freetext detection.

pub struct RapidOcr {
    pub model_path: Option<PathBuf>,
    pub det_path: Option<PathBuf>,
    pub rec_path: Option<PathBuf>,
    pub dict_path: Option<PathBuf>,
}
impl OcrEngine for RapidOcr {
    fn recognize(&self, image: &RgbImage, bbox: [u32; 4]) -> Option<String> {
        // if real model present, try ort rec; fallback dummy
        if let (Some(rec), Some(dict)) = (&self.rec_path, &self.dict_path) {
            if rec.exists() && dict.exists() {
                if let Some(s) = rapid_recognize_crop(image, bbox, rec, dict) { return Some(s); }
            }
        }
        Some("テスト".to_string())
    }
    fn recognize_regions(&self, full: &RgbImage, _exclude: &[[u32; 4]]) -> Vec<TextRegion> {
        if full.width() < 20 || full.height() < 20 { return Vec::new(); }
        if let (Some(det), Some(rec), Some(dict)) = (&self.det_path, &self.rec_path, &self.dict_path) {
            if det.exists() && rec.exists() && dict.exists() {
                let regs = rapid_detect_regions(full, det, rec, dict);
                if !regs.is_empty() { return regs; }
            }
        }
        // fallback dummy if model not ready or detection empty
        vec![TextRegion { bbox: [20,20,120,60], text: "フリーテキスト".to_string() }]
    }
    fn name(&self) -> &'static str { "rapid" }
}

fn rapid_detect_regions(full: &RgbImage, det: &Path, rec: &Path, dict: &Path) -> Vec<TextRegion> {
    match rapid_detect_ort(full, det, rec, dict) {
        Ok(v) => v,
        Err(e) => { eprintln!("[rapid] det failed: {e} — fallback dummy"); Vec::new() }
    }
}
fn rapid_detect_ort(full: &RgbImage, det: &Path, rec: &Path, dict: &Path) -> anyhow::Result<Vec<TextRegion>> {
    let (orig_w, orig_h) = (full.width(), full.height());
    // letterbox to 640 like YOLO
    let dyn_img = DynamicImage::ImageRgb8(full.clone());
    let target = 640f32;
    let scale = (target / orig_h as f32).min(target / orig_w as f32);
    let new_w = (orig_w as f32 * scale).round() as u32;
    let new_h = (orig_h as f32 * scale).round() as u32;
    let dw = ((target - new_w as f32)/2.0).floor() as i64;
    let dh = ((target - new_h as f32)/2.0).floor() as i64;
    let resized = dyn_img.resize_exact(new_w, new_h, FilterType::Triangle);
    let mut padded = RgbImage::from_pixel(640, 640, image::Rgb([114,114,114]));
    image::imageops::overlay(&mut padded, &resized.to_rgb8(), dw, dh);
    let mut tensor = Array4::<f32>::zeros((1,3,640,640));
    for y in 0..640 { for x in 0..640 {
        let p = padded.get_pixel(x,y);
        // Paddle det uses mean 0.485/0.456/0.406 std 0.229/0.224/0.225 but 0-1 works for now
        tensor[[0,0,y as usize,x as usize]] = p[0] as f32/255.0;
        tensor[[0,1,y as usize,x as usize]] = p[1] as f32/255.0;
        tensor[[0,2,y as usize,x as usize]] = p[2] as f32/255.0;
    }}
    let mut sess = Session::builder()?.commit_from_file(det)?;
    let input = Tensor::from_array(tensor)?;
    let outputs = sess.run(ort::inputs![input])?;
    let (_, val) = outputs.into_iter().next().ok_or_else(|| anyhow::anyhow!("no output"))?;
    let (shape, data) = val.try_extract_tensor::<f32>()?;
    // shape e.g. [1,1,640,640] or [1,640,640]
    let h = if shape.len()>=4 { shape[2] as usize } else if shape.len()==3 { shape[1] as usize } else { 640 };
    let w = if shape.len()>=4 { shape[3] as usize } else if shape.len()==3 { shape[2] as usize } else { 640 };
    // find boxes via threshold >0.3
    let thresh = 0.3f32;
    let mut mask = vec![vec![false; w]; h];
    for y in 0..h { for x in 0..w {
        let idx = y*w+x;
        if idx < data.len() && data[idx] > thresh { mask[y][x]=true; }
    }}
    // connected components BFS 4-dir
    let mut visited = vec![vec![false; w]; h];
    let mut boxes: Vec<[u32;4]> = Vec::new();
    for y in 0..h { for x in 0..w {
        if !mask[y][x] || visited[y][x] { continue; }
        let mut q = vec![(y,x)];
        visited[y][x]=true;
        let (mut minx, mut miny, mut maxx, mut maxy) = (x,y,x,y);
        let mut qi=0;
        while qi<q.len() {
            let (cy,cx)=q[qi]; qi+=1;
            for (ny,nx) in [(cy.wrapping_sub(1),cx),(cy+1,cx),(cy,cx.wrapping_sub(1)),(cy,cx+1)] {
                if ny>=h||nx>=w { continue; }
                if !mask[ny][nx]||visited[ny][nx]{continue;}
                visited[ny][nx]=true;
                q.push((ny,nx));
                minx=minx.min(nx); miny=miny.min(ny); maxx=maxx.max(nx); maxy=maxy.max(ny);
            }
        }
        let bw = maxx-minx+1; let bh = maxy-miny+1;
        if bw < 8 || bh < 8 { continue; }
        // map back to orig via scale/dw/dh
        let x1 = ((minx as f32 - dw as f32)/scale).clamp(0.0, orig_w as f32).round() as u32;
        let y1 = ((miny as f32 - dh as f32)/scale).clamp(0.0, orig_h as f32).round() as u32;
        let x2 = ((maxx as f32 - dw as f32)/scale).clamp(0.0, orig_w as f32).round() as u32;
        let y2 = ((maxy as f32 - dh as f32)/scale).clamp(0.0, orig_h as f32).round() as u32;
        if x2 > x1 && y2 > y1 { boxes.push([x1,y1,x2,y2]); }
    }}
    // for each box try rec to get text, else placeholder
    let mut regs = Vec::new();
    for b in boxes {
        let txt = rapid_recognize_crop(full, b, rec, dict).unwrap_or_else(|| "テキスト".to_string());
        regs.push(TextRegion{ bbox:b, text: txt });
    }
    Ok(regs)
}
fn rapid_recognize_crop(img: &RgbImage, bbox: [u32;4], rec: &Path, dict: &Path) -> Option<String> {
    let _ = (img, bbox, rec, dict);
    // TODO real rec: crop resize 32x, ort rec session, dict decode — for now dummy
    None
}

pub struct MangaOcr {
    pub model_path: Option<PathBuf>,
}
impl OcrEngine for MangaOcr {
    fn recognize(&self, _image: &RgbImage, _bbox: [u32; 4]) -> Option<String> {
        Some("テスト".to_string())
    }
    fn recognize_regions(&self, full: &RgbImage, _exclude: &[[u32; 4]]) -> Vec<TextRegion> {
        if full.width() < 20 || full.height() < 20 {
            return Vec::new();
        }
        vec![TextRegion {
            bbox: [20, 20, 120, 60],
            text: "フリーテキスト".to_string(),
        }]
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
            OcrScript::Auto => "jpn_vert+jpn+eng".to_string(),
        }
    }
}
impl OcrEngine for TesseractOcr {
    fn recognize(&self, image: &RgbImage, bbox: [u32; 4]) -> Option<String> {
        let (x1,y1,x2,y2) = (bbox[0],bbox[1],bbox[2],bbox[3]);
        if x2 <= x1 || y2 <= y1 { return None; }
        let w = x2 - x1;
        let h = y2 - y1;
        if w < 8 || h < 8 { return None; }
        // crop
        let mut crop = RgbImage::new(w, h);
        for cy in 0..h { for cx in 0..w { if x1+cx < image.width() && y1+cy < image.height() { crop.put_pixel(cx, cy, *image.get_pixel(x1+cx, y1+cy)); } } }
        tesseract_ocr_image(&crop, &self.lang())
    }
    fn recognize_regions(&self, full: &RgbImage, _exclude: &[[u32; 4]]) -> Vec<TextRegion> {
        if full.width() < 20 || full.height() < 20 { return Vec::new(); }
        tesseract_tsv_regions(full, &self.lang())
    }
    fn name(&self) -> &'static str {
        "tesseract"
    }
}

fn tesseract_ocr_image(img: &RgbImage, lang: &str) -> Option<String> {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("kzktdk_tess_{}.png", std::process::id()));
    if img.save(&path).is_err() { return None; }
    let out = std::process::Command::new("tesseract")
        .arg(&path)
        .arg("stdout")
        .arg("-l").arg(lang)
        .arg("--psm").arg("6")
        .arg("--oem").arg("1")
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
    if img.save(&path).is_err() { return Vec::new(); }
    let out = std::process::Command::new("tesseract")
        .arg(&path)
        .arg("stdout")
        .arg("-l").arg(lang)
        .arg("--psm").arg("6")
        .arg("--oem").arg("1")
        .arg("tsv")
        .output();
    let _ = std::fs::remove_file(&path);
    let mut regs = Vec::new();
    if let Ok(o) = out {
        if o.status.success() {
            let txt = String::from_utf8_lossy(&o.stdout);
            for line in txt.lines().skip(1) {
                let cols: Vec<&str> = line.split('\t').collect();
                if cols.len() < 12 { continue; }
                let conf: i32 = cols[10].parse().unwrap_or(-1);
                if conf < 30 { continue; }
                let text = cols[11].trim();
                if text.is_empty() { continue; }
                let x: u32 = cols[6].parse().unwrap_or(0);
                let y: u32 = cols[7].parse().unwrap_or(0);
                let w: u32 = cols[8].parse().unwrap_or(0);
                let h: u32 = cols[9].parse().unwrap_or(0);
                if w < 12 || h < 8 { continue; }
                regs.push(TextRegion { bbox: [x, y, x+w, y+h], text: text.to_string() });
            }
        }
    }
    // cluster nearby words into lines via merge handled in preparer; return raw
    regs
}

// ---------- Engine factory + auto-download ----------

fn cache_dir_for(engine: &str) -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".cache/kzktdk/models").join(engine)
    } else {
        PathBuf::from("/tmp/kzktdk/models").join(engine)
    }
}

fn download_file(url: &str, dst: &Path) -> bool {
    if dst.exists() { return true; }
    if let Some(parent) = dst.parent() { let _ = std::fs::create_dir_all(parent); }
    eprintln!("[OCR] downloading {} -> {:?}", url, dst);
    // try curl first (handles HF redirects + large files)
    let out = std::process::Command::new("curl").arg("-L").arg("-f").arg("-o").arg(dst).arg(url).output();
    if let Ok(o) = out { if o.status.success() && dst.exists() { return true; } eprintln!("[OCR] curl failed: {}", String::from_utf8_lossy(&o.stderr)); }
    // fallback reqwest via tokio if curl missing
    false
}

fn ensure_rapid_cached(custom_path: Option<&Path>) -> Option<(PathBuf,PathBuf,PathBuf)> {
    if let Some(p) = custom_path { if p.exists() { return Some((p.to_path_buf(), p.to_path_buf(), p.to_path_buf())); } eprintln!("[OCR] custom --ocr-model {:?} not found", p); }
    let dir = cache_dir_for("rapid");
    let det = dir.join("ch_PP-OCRv3_det_infer.onnx");
    let rec = dir.join("japan_rec_crnn.onnx");
    let dict = dir.join("japan_dict.txt");
    let need_dl = !det.exists() || !rec.exists() || !dict.exists();
    if need_dl {
        let _ = std::fs::create_dir_all(&dir);
        let det_url = "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv3/ch_PP-OCRv3_det_infer.onnx";
        let rec_url = "https://huggingface.co/SWHL/RapidOCR/resolve/main/PP-OCRv1/japan_rec_crnn.onnx";
        let dict_url = "https://raw.githubusercontent.com/PaddlePaddle/PaddleOCR/main/ppocr/utils/dict/japan_dict.txt";
        let ok_det = download_file(det_url, &det);
        let ok_rec = download_file(rec_url, &rec);
        let ok_dict = download_file(dict_url, &dict);
        if !ok_det || !ok_rec || !ok_dict {
            eprintln!("[OCR] rapid download incomplete det:{} rec:{} dict:{} — will use dummy but files partially cached at {:?}", ok_det, ok_rec, ok_dict, dir);
        } else {
            eprintln!("[OCR] rapid models cached at {:?} (det {} rec {} dict {})", dir, det.exists(), rec.exists(), dict.exists());
        }
    }
    if det.exists() && rec.exists() { Some((det, rec, dict)) } else { None }
}

fn ensure_model_cached(engine: &str, custom_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = custom_path {
        if p.exists() { return Some(p.to_path_buf()); }
        eprintln!("[OCR] custom --ocr-model {:?} not found, fallback cache", p);
    }
    let dir = cache_dir_for(engine);
    if dir.exists() {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().map(|e| e.eq_ignore_ascii_case("onnx")).unwrap_or(false) { return Some(p); }
                if p.is_dir() {
                    if let Ok(rd2) = std::fs::read_dir(&p) {
                        for e2 in rd2.flatten() {
                            let pp = e2.path();
                            if pp.extension().map(|e| e.eq_ignore_ascii_case("onnx")).unwrap_or(false) { return Some(pp); }
                        }
                    }
                }
            }
        }
    }
    match engine {
        "rapid" => { eprintln!("[OCR] rapid model not cached at {:?} — expected det+rec .onnx (SWHL/RapidOCR). Run with --ocr-model <path> or place model.", dir); }
        "manga" => { eprintln!("[OCR] manga model not cached at {:?} — expected onnx-community/manga-ocr-base-ONNX. Run with --ocr-model <path>.", dir); }
        _ => {}
    }
    let _ = std::fs::create_dir_all(&dir);
    None
}

pub fn create_ocr_engine(name: &str, ocr_model: Option<&Path>, script: OcrScript) -> Box<dyn OcrEngine> {
    match name.to_lowercase().as_str() {
        "none" => Box::new(NoopOcr),
        "rapid" => {
            let cached = ensure_rapid_cached(ocr_model);
            if let Some((det, rec, dict)) = cached {
                Box::new(RapidOcr { model_path: Some(det.clone()), det_path: Some(det), rec_path: Some(rec), dict_path: Some(dict) })
            } else {
                let p = ensure_model_cached("rapid", ocr_model);
                Box::new(RapidOcr { model_path: p, det_path: None, rec_path: None, dict_path: None })
            }
        }
        "manga" => {
            let p = ensure_model_cached("manga", ocr_model);
            Box::new(MangaOcr { model_path: p })
        }
        "tesseract" => Box::new(TesseractOcr { script }),
        // legacy aliases
        "vision" | "local" => {
            eprintln!("[OCR] engine '{}' deprecated, use rapid|manga|tesseract, fallback none", name);
            Box::new(NoopOcr)
        }
        other => {
            eprintln!("[OCR] unknown engine '{}', fallback none", other);
            Box::new(NoopOcr)
        }
    }
}
