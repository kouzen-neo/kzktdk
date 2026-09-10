use anyhow::{Context, Result};
use image::{Rgb, RgbImage};
use std::path::{Path, PathBuf};

use kzktdk::archive::{self, PreparedInput};
use kzktdk::metadata;
use kzktdk::model::yolo::YoloModel;

use super::util::{ensure_model, file_name, parse_jobs};

#[allow(clippy::too_many_arguments)]
pub async fn run(
    input: PathBuf,
    output: Option<PathBuf>,
    model: PathBuf,
    json: Option<PathBuf>,
    jobs: String,
    translate_free_text: bool,
    ocr_script: String,
    ocr: String,
    ocr_model: Option<PathBuf>,
    format: String,
    quiet: bool,
) -> Result<()> {
    let as_json_d = format == "json";
    macro_rules! dinfo {
        ($($t:tt)*) => {
            if as_json_d || quiet { eprintln!($($t)*) } else { println!($($t)*) }
        };
    }
    // Machine-readable detection record shared by single + batch branches.
    let det_record = |name: &str,
                      w: u32,
                      h: u32,
                      dets: &[kzktdk::model::yolo::Detection],
                      ft: &[[u32; 4]]|
     -> serde_json::Value {
        serde_json::json!({
            "page": name,
            "width": w,
            "height": h,
            "bubbles": dets.iter().enumerate().map(|(i, d)| serde_json::json!({
                "id": (i + 1).to_string(),
                "bbox": [d.x1, d.y1, d.x2, d.y2],
                "conf": d.conf,
            })).collect::<Vec<_>>(),
            "freetext": ft.iter().enumerate().map(|(i, f)| serde_json::json!({
                "id": format!("ft{}", i + 1),
                "bbox": f,
            })).collect::<Vec<_>>(),
        })
    };
    let mut json_pages: Vec<serde_json::Value> = Vec::new();
    // Fail fast: stub OCR engines cannot read text (never guess).
    if translate_free_text && ocr != "none" {
        kzktdk::ocr::ensure_rec_available(&ocr, "--translate-free-text")?;
    }
    let model_file = ensure_model(&model)?;
    dinfo!("[1/2] Loading model from {:?}...", model_file);
    let mut yolo = YoloModel::new(&model_file)?;
    let jobs_num = parse_jobs(&jobs);

    let prepared = archive::prepare_input(&input)?;
    match prepared {
        PreparedInput::SingleImage(img_path) => {
            let out_path = output
                .clone()
                .unwrap_or_else(|| PathBuf::from("detected.png"));
            // If output is a directory, place file inside
            let out_path = if out_path.is_dir() || out_path.to_string_lossy().ends_with('/') {
                std::fs::create_dir_all(&out_path)?;
                out_path.join(file_name(img_path.as_path())?)
            } else {
                if let Some(parent) = out_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                out_path
            };
            dinfo!("[2/2] Detecting bubbles on {:?}...", img_path);
            let img =
                image::open(&img_path).with_context(|| format!("Failed to open {:?}", img_path))?;
            let (w, h) = (img.width(), img.height());
            let mut detections = yolo.detect_bubbles(&img)?;
            dinfo!("Found {} speech bubbles.", detections.len());
            // freetext
            let mut ft_boxes: Vec<[u32; 4]> = Vec::new();
            if translate_free_text {
                if ocr == "none" {
                    eprintln!("[freetext] butuh --ocr rapid|manga|tesseract, fallback bubble-only");
                } else {
                    let script = kzktdk::ocr::OcrScript::from_key(&ocr_script);
                    let engine = kzktdk::ocr::create_ocr_engine(&ocr, ocr_model.as_deref(), script);
                    if engine.name() != "none" {
                        let bubble_boxes: Vec<[u32; 4]> = detections
                            .iter()
                            .map(|d| [d.x1, d.y1, d.x2, d.y2])
                            .collect();
                        ft_boxes = kzktdk::preparer::detect_free_text(
                            &img.to_rgb8(),
                            &bubble_boxes,
                            engine.as_ref(),
                        );
                        dinfo!("Found {} freetext regions.", ft_boxes.len());
                    }
                }
            }
            if as_json_d {
                json_pages.push(det_record(
                    &img_path.file_name().unwrap_or_default().to_string_lossy(),
                    w,
                    h,
                    &detections,
                    &ft_boxes,
                ));
            }
            let mut rgb_img = img.to_rgb8();
            for (idx, det) in detections.iter().enumerate() {
                dinfo!(
                    "  Bubble #{}: [{}, {}, {}, {}] (conf: {:.2})",
                    idx + 1,
                    det.x1,
                    det.y1,
                    det.x2,
                    det.y2,
                    det.conf
                );
                draw_rect(
                    &mut rgb_img,
                    det.x1,
                    det.y1,
                    det.x2,
                    det.y2,
                    Rgb([255, 0, 0]),
                    2,
                );
            }
            for (idx, fb) in ft_boxes.iter().enumerate() {
                dinfo!(
                    "  Freetext #{}: [{},{},{},{}]",
                    idx + 1,
                    fb[0],
                    fb[1],
                    fb[2],
                    fb[3]
                );
                draw_rect(
                    &mut rgb_img,
                    fb[0],
                    fb[1],
                    fb[2],
                    fb[3],
                    Rgb([0, 200, 0]),
                    2,
                );
            }
            rgb_img.save(&out_path)?;
            dinfo!("Saved detection preview to {:?}", out_path);
            if let Some(json_path) = json {
                let mut bubbles: Vec<metadata::Bubble> = detections
                    .iter()
                    .enumerate()
                    .map(|(i, d)| {
                        metadata::Bubble::detected(
                            (i + 1).to_string(),
                            [d.x1, d.y1, d.x2, d.y2],
                            d.conf,
                        )
                    })
                    .collect();
                for (i, fb) in ft_boxes.iter().enumerate() {
                    bubbles.push(metadata::Bubble::detected(
                        format!("ft{}", i + 1),
                        *fb,
                        0.90,
                    ));
                }
                let mut data = metadata::PageEditData::new(
                    file_name(img_path.as_path())?.to_string_lossy().to_string(),
                    w,
                    h,
                    "English".to_string(),
                    "classic".to_string(),
                    bubbles,
                );
                if ocr != "none" {
                    data.ocr_engine = Some(ocr.clone());
                }
                metadata::save_page_metadata(&json_path, &data)?;
                dinfo!("Saved JSON to {:?}", json_path);
            }
            if as_json_d {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json_pages).context("serialize detections")?
                );
            }
        }
        PreparedInput::Batch {
            images,
            _temp_guard: _,
            is_archive: _,
            is_pdf: _,
            original_name,
        } => {
            let out_dir = output.unwrap_or_else(|| PathBuf::from("detected"));
            std::fs::create_dir_all(&out_dir)?;
            dinfo!(
                "[2/2] Detecting bubbles on {} pages (jobs={})...",
                images.len(),
                jobs_num
            );
            let mut all_pages = Vec::new();
            let make_page_with_ft = |p: &PathBuf,
                                     dets: Vec<kzktdk::model::yolo::Detection>,
                                     w: u32,
                                     h: u32,
                                     rgb_ref: &RgbImage|
             -> Result<metadata::PageEditData> {
                let mut ft_boxes: Vec<[u32; 4]> = Vec::new();
                if translate_free_text && ocr != "none" {
                    let script = kzktdk::ocr::OcrScript::from_key(&ocr_script);
                    let engine = kzktdk::ocr::create_ocr_engine(&ocr, ocr_model.as_deref(), script);
                    if engine.name() != "none" {
                        let bboxes: Vec<[u32; 4]> =
                            dets.iter().map(|d| [d.x1, d.y1, d.x2, d.y2]).collect();
                        ft_boxes =
                            kzktdk::preparer::detect_free_text(rgb_ref, &bboxes, engine.as_ref());
                    }
                }
                let mut bubbles: Vec<metadata::Bubble> = dets
                    .iter()
                    .enumerate()
                    .map(|(i, d)| {
                        metadata::Bubble::detected(
                            (i + 1).to_string(),
                            [d.x1, d.y1, d.x2, d.y2],
                            d.conf,
                        )
                    })
                    .collect();
                for (i, fb) in ft_boxes.iter().enumerate() {
                    bubbles.push(metadata::Bubble::detected(
                        format!("ft{}", i + 1),
                        *fb,
                        0.90,
                    ));
                }
                let mut data = metadata::PageEditData::new(
                    file_name(p)?.to_string_lossy().to_string(),
                    w,
                    h,
                    "English".to_string(),
                    "classic".to_string(),
                    bubbles,
                );
                if ocr != "none" {
                    data.ocr_engine = Some(ocr.clone());
                }
                Ok(data)
            };
            if jobs_num <= 1 {
                for (idx, p) in images.iter().enumerate() {
                    let img = image::open(p).with_context(|| format!("Failed to open {:?}", p))?;
                    let (w, h) = (img.width(), img.height());
                    let dets = yolo.detect_bubbles(&img)?;
                    dinfo!(
                        "[Page {}/{}] {:?}: {} bubbles",
                        idx + 1,
                        images.len(),
                        file_name(p)?,
                        dets.len()
                    );
                    let rgb = img.to_rgb8();
                    let mut rgb_out = rgb.clone();
                    for d in &dets {
                        draw_rect(&mut rgb_out, d.x1, d.y1, d.x2, d.y2, Rgb([255, 0, 0]), 2);
                    }
                    // also draw ft
                    let mut fts_page: Vec<[u32; 4]> = Vec::new();
                    if translate_free_text && ocr != "none" {
                        let script = kzktdk::ocr::OcrScript::from_key(&ocr_script);
                        let engine =
                            kzktdk::ocr::create_ocr_engine(&ocr, ocr_model.as_deref(), script);
                        if engine.name() != "none" {
                            let bboxes: Vec<[u32; 4]> =
                                dets.iter().map(|d| [d.x1, d.y1, d.x2, d.y2]).collect();
                            fts_page =
                                kzktdk::preparer::detect_free_text(&rgb, &bboxes, engine.as_ref());
                            for fb in &fts_page {
                                draw_rect(
                                    &mut rgb_out,
                                    fb[0],
                                    fb[1],
                                    fb[2],
                                    fb[3],
                                    Rgb([0, 200, 0]),
                                    2,
                                );
                            }
                        }
                    }
                    let out = out_dir.join(file_name(p)?);
                    rgb_out.save(&out)?;
                    if as_json_d {
                        json_pages.push(det_record(
                            &p.file_name().unwrap_or_default().to_string_lossy(),
                            w,
                            h,
                            &dets,
                            &fts_page,
                        ));
                    }
                    if json.is_some() {
                        all_pages.push(make_page_with_ft(p, dets, w, h, &rgb)?);
                    }
                }
            } else {
                // For batch detect, sequential yolo is fine; parallel would need Send. Keep sequential for now with progress.
                for (idx, p) in images.iter().enumerate() {
                    let img = image::open(p).with_context(|| format!("Failed to open {:?}", p))?;
                    let (w, h) = (img.width(), img.height());
                    let dets = yolo.detect_bubbles(&img)?;
                    dinfo!(
                        "[Page {}/{}] {:?}: {} bubbles",
                        idx + 1,
                        images.len(),
                        file_name(p)?,
                        dets.len()
                    );
                    let rgb = img.to_rgb8();
                    let mut rgb_out = rgb.clone();
                    for d in &dets {
                        draw_rect(&mut rgb_out, d.x1, d.y1, d.x2, d.y2, Rgb([255, 0, 0]), 2);
                    }
                    let mut fts_page: Vec<[u32; 4]> = Vec::new();
                    if translate_free_text && ocr != "none" {
                        let script = kzktdk::ocr::OcrScript::from_key(&ocr_script);
                        let engine =
                            kzktdk::ocr::create_ocr_engine(&ocr, ocr_model.as_deref(), script);
                        if engine.name() != "none" {
                            let bboxes: Vec<[u32; 4]> =
                                dets.iter().map(|d| [d.x1, d.y1, d.x2, d.y2]).collect();
                            fts_page =
                                kzktdk::preparer::detect_free_text(&rgb, &bboxes, engine.as_ref());
                            for fb in &fts_page {
                                draw_rect(
                                    &mut rgb_out,
                                    fb[0],
                                    fb[1],
                                    fb[2],
                                    fb[3],
                                    Rgb([0, 200, 0]),
                                    2,
                                );
                            }
                        }
                    }
                    let out = out_dir.join(file_name(p)?);
                    rgb_out.save(&out)?;
                    if as_json_d {
                        json_pages.push(det_record(
                            &p.file_name().unwrap_or_default().to_string_lossy(),
                            w,
                            h,
                            &dets,
                            &fts_page,
                        ));
                    }
                    if json.is_some() {
                        all_pages.push(make_page_with_ft(p, dets, w, h, &rgb)?);
                    }
                }
            }
            dinfo!("Saved {} previews to {:?}", images.len(), out_dir);
            if let Some(json_path) = json {
                let v = serde_json::to_value(&all_pages).context("serialize batch pages")?;
                std::fs::create_dir_all(json_path.parent().unwrap_or(Path::new(".")))?;
                std::fs::write(
                    &json_path,
                    serde_json::to_string_pretty(&v).context("serialize batch JSON")?,
                )?;
                dinfo!("Saved batch JSON to {:?}", json_path);
            }
            if as_json_d {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json_pages).context("serialize detections")?
                );
            }
        }
    }
    Ok(())
}

fn draw_rect(
    img: &mut RgbImage,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    color: Rgb<u8>,
    thickness: u32,
) {
    let (w, h) = img.dimensions();
    for t in 0..thickness {
        // Horizontal top & bottom
        for x in x1..=x2 {
            if y1 + t < h && x < w {
                img.put_pixel(x, y1 + t, color);
            }
            if y2 >= t && y2 - t < h && x < w {
                img.put_pixel(x, y2 - t, color);
            }
        }
        // Vertical left & right
        for y in y1..=y2 {
            if x1 + t < w && y < h {
                img.put_pixel(x1 + t, y, color);
            }
            if x2 >= t && x2 - t < w && y < h {
                img.put_pixel(x2 - t, y, color);
            }
        }
    }
}
