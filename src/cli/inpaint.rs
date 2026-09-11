use anyhow::Result;
use std::path::PathBuf;

use kzktdk::inpaint::inpaint_image;
use kzktdk::model::yolo::YoloModel;

use super::util::ensure_model;

pub async fn run(input: PathBuf, output: PathBuf, model: PathBuf) -> Result<()> {
    let model_file = ensure_model(&model)?;
    println!("[1/3] Loading YOLO model...");
    let mut yolo = YoloModel::new(&model_file)?;

    println!("[2/3] Detecting bubbles on {:?}...", input);
    let img = image::open(&input)?;
    let detections = yolo.detect_bubbles(&img)?;
    println!("Found {} bubbles.", detections.len());

    println!("[3/3] Inpainting dialogue strokes with clean manga contouring (parallel)...");
    let mut rgb_img = img.to_rgb8();
    inpaint_image(&mut rgb_img, &detections)?;

    rgb_img.save(&output)?;
    println!("Inpainting complete! Saved clean page to {:?}", output);
    Ok(())
}
