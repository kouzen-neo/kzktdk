use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BubbleStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_family: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_color: Option<[u8;3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<[u8;3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bubble {
    pub id: String,
    pub bbox: [u32; 4],
    pub conf: f32,
    pub translated: String,
    pub bg_color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<BubbleStyle>,
    #[serde(default)]
    pub edited: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageEditData {
    pub version: u32,
    pub page: String,
    pub width: u32,
    pub height: u32,
    pub target_lang: String,
    pub prompt_sig: String,
    pub bubbles: Vec<Bubble>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub pages: Vec<String>,
    pub target_lang: Option<String>,
}

impl PageEditData {
    pub fn new(
        page: String,
        width: u32,
        height: u32,
        target_lang: String,
        prompt_sig: String,
        bubbles: Vec<Bubble>,
    ) -> Self {
        Self {
            version: 1,
            page,
            width,
            height,
            target_lang,
            prompt_sig,
            bubbles,
        }
    }
}

fn atomic_write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create dir {:?}", parent))?;
    }
    let tmp = path.with_extension(format!("tmp_{}", std::process::id()));
    let file = std::fs::File::create(&tmp)
        .with_context(|| format!("Failed to create tmp {:?}", tmp))?;
    serde_json::to_writer_pretty(file, value).context("Failed to write JSON")?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("Failed to rename {:?} -> {:?}", tmp, path))?;
    Ok(())
}

pub fn save_page_metadata(path: &Path, data: &PageEditData) -> Result<()> {
    let v = serde_json::to_value(data).context("Serialize PageEditData")?;
    atomic_write_json(path, &v)
}

pub fn load_page_metadata(path: &Path) -> Result<PageEditData> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("Failed to open metadata {:?}", path))?;
    let data: PageEditData =
        serde_json::from_reader(file).with_context(|| format!("Failed to parse {:?}", path))?;
    Ok(data)
}

pub fn save_project(project_path: &Path, page_jsons: &[PathBuf], target_lang: Option<String>) -> Result<()> {
    let pages: Vec<String> = page_jsons
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let proj = Project {
        version: 1,
        pages,
        target_lang,
    };
    let v = serde_json::to_value(&proj).context("Serialize Project")?;
    atomic_write_json(project_path, &v)
}

pub fn load_project(project_path: &Path) -> Result<Project> {
    let file = std::fs::File::open(project_path)
        .with_context(|| format!("Failed to open project {:?}", project_path))?;
    let proj: Project = serde_json::from_reader(file).context("Parse Project")?;
    Ok(proj)
}

/// Build PageEditData from detections + translations
pub fn build_page_data(
    page_name: &str,
    width: u32,
    height: u32,
    target_lang: &str,
    prompt_sig: &str,
    detections: &[crate::model::yolo::Detection],
    translations: &HashMap<String, String>,
    bg_colors: &HashMap<String, [u8; 3]>,
) -> PageEditData {
    let mut bubbles = Vec::new();
    for (i, det) in detections.iter().enumerate() {
        let id = (i + 1).to_string();
        let translated = translations.get(&id).cloned().unwrap_or_default();
        let bg = bg_colors.get(&id).copied();
        bubbles.push(Bubble {
            id,
            bbox: [det.x1, det.y1, det.x2, det.y2],
            conf: det.conf,
            translated,
            bg_color: bg,
            style: None,
            edited: false,
        });
    }
    PageEditData::new(
        page_name.to_string(),
        width,
        height,
        target_lang.to_string(),
        prompt_sig.to_string(),
        bubbles,
    )
}
