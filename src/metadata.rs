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
    pub text_color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<[u8; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_bold: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_italic: Option<bool>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask_path: Option<String>,
}

impl Bubble {
    /// Freshly-detected bubble: id/bbox/conf set, everything else empty.
    ///
    /// Shared constructor for all detection sites (replaces ~10 copied
    /// literals). Bubbles carrying real content (`translated`, `raw_text`)
    /// keep explicit literals so the difference stays visible.
    pub fn detected(id: String, bbox: [u32; 4], conf: f32) -> Self {
        Self {
            id,
            bbox,
            conf,
            translated: String::new(),
            bg_color: None,
            style: None,
            edited: false,
            raw_text: None,
            mask_path: None,
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub version: u32,
    pub pages: Vec<String>,
    pub target_lang: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ocr_engine: Option<String>,
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
            ocr_engine: None,
            order: None,
        }
    }

    /// Bubbles in reading order if `order` is set, else in stored order.
    pub fn ordered_bubbles(&self) -> Vec<&Bubble> {
        if let Some(order) = &self.order {
            let map: std::collections::HashMap<&str, &Bubble> =
                self.bubbles.iter().map(|b| (b.id.as_str(), b)).collect();
            let mut out = Vec::with_capacity(self.bubbles.len());
            for id in order {
                if let Some(b) = map.get(id.as_str()) {
                    out.push(*b);
                }
            }
            // Append any bubbles missing from order (backward compat).
            for b in &self.bubbles {
                if !order.iter().any(|id| id == &b.id) {
                    out.push(b);
                }
            }
            out
        } else {
            self.bubbles.iter().collect()
        }
    }
}

/// One validation issue found in a metadata file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationError {
    pub id: Option<String>,
    pub code: String,
    pub msg: String,
}

/// Machine-readable validation report (`--format json`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub kind: String,
    pub errors: Vec<ValidationError>,
    pub meta: serde_json::Value,
}

/// Validate a PageEditData, returning a list of issues (empty = valid).
pub fn validate_page(data: &PageEditData) -> Vec<ValidationError> {
    let mut errors = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for b in &data.bubbles {
        if !seen.insert(&b.id) {
            errors.push(ValidationError {
                id: Some(b.id.clone()),
                code: "duplicate_id".to_string(),
                msg: format!("Duplicate id {}", b.id),
            });
        }
        if b.bbox[0] >= b.bbox[2] || b.bbox[1] >= b.bbox[3] {
            errors.push(ValidationError {
                id: Some(b.id.clone()),
                code: "invalid_bbox".to_string(),
                msg: format!("Invalid bbox {:?}: require x1<x2 and y1<y2", b.bbox),
            });
        }
        if b.bbox[2] > data.width || b.bbox[3] > data.height {
            errors.push(ValidationError {
                id: Some(b.id.clone()),
                code: "out_of_bounds".to_string(),
                msg: format!(
                    "Bbox {:?} out of bounds {}x{}",
                    b.bbox, data.width, data.height
                ),
            });
        }
    }
    if let Some(order) = &data.order {
        let ids: std::collections::HashSet<&str> =
            data.bubbles.iter().map(|b| b.id.as_str()).collect();
        for id in order {
            if !ids.contains(id.as_str()) {
                errors.push(ValidationError {
                    id: Some(id.clone()),
                    code: "unknown_order_id".to_string(),
                    msg: format!("Order id '{}' not found in bubbles", id),
                });
            }
        }
    }
    errors
}

/// Acquire a sibling `.lock` file with `create_new` + retry.
/// Returns the lock path (to be removed after the critical section).
fn acquire_lock(path: &Path) -> Result<PathBuf> {
    let lock = path.with_extension(format!(
        "{}lock",
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| format!("{}.", e))
            .unwrap_or_default()
    ));
    // Fallback: if extension juggling looks odd, use simple `.lock` sibling.
    let lock = if lock == path {
        path.with_extension("lock")
    } else {
        lock
    };
    for _ in 0..crate::config::METADATA_LOCK_SPIN_ATTEMPTS {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock)
        {
            Ok(_) => return Ok(lock),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                std::thread::sleep(std::time::Duration::from_millis(
                    crate::config::METADATA_LOCK_POLL_MS,
                ));
                continue;
            }
            Err(e) => {
                return Err(e).with_context(|| format!("Failed to acquire lock {:?}", lock));
            }
        }
    }
    anyhow::bail!("Timed out acquiring lock {:?}", lock)
}

fn atomic_write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create dir {:?}", parent))?;
    }
    // Serialize first so validation/serialization errors never leave a lock behind.
    let pretty = serde_json::to_string_pretty(value).context("Failed to serialize JSON")?;
    // File-lock against concurrent GUI-vs-CLI writers.
    let lock = acquire_lock(path)?;
    let res: Result<()> = (|| {
        // Backup previous version (max 3, lightweight undo)
        if path.exists() {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let bak = path.with_extension(format!("bak.{}", ts));
            let _ = std::fs::copy(path, &bak);
            // Prune old backups keep 3
            if let Some(parent) = path.parent() {
                if let Some(stem) = path.file_name().and_then(|s| s.to_str()) {
                    if let Ok(entries) = std::fs::read_dir(parent) {
                        let mut baks: Vec<_> = entries
                            .filter_map(|e| e.ok())
                            .filter(|e| {
                                e.file_name()
                                    .to_str()
                                    .map(|n| n.starts_with(&format!("{}.bak.", stem)))
                                    .unwrap_or(false)
                            })
                            .collect();
                        baks.sort_by_key(|e| e.path());
                        while baks.len() > 3 {
                            if let Some(old) = baks.first() {
                                let _ = std::fs::remove_file(old.path());
                                baks.remove(0);
                            } else {
                                break;
                            }
                        }
                    }
                }
            }
        }
        let tmp = path.with_extension(format!("tmp_{}", std::process::id()));
        std::fs::write(&tmp, &pretty).with_context(|| format!("Failed to write tmp {:?}", tmp))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("Failed to rename {:?} -> {:?}", tmp, path))?;
        Ok(())
    })();
    let _ = std::fs::remove_file(&lock);
    res
}

pub fn save_page_metadata(path: &Path, data: &PageEditData) -> Result<()> {
    let v = serde_json::to_value(data).context("Serialize PageEditData")?;
    atomic_write_json(path, &v)
}

pub fn load_page_metadata(path: &Path) -> Result<PageEditData> {
    let file =
        std::fs::File::open(path).with_context(|| format!("Failed to open metadata {:?}", path))?;
    let data: PageEditData =
        serde_json::from_reader(file).with_context(|| format!("Failed to parse {:?}", path))?;
    Ok(data)
}

pub fn save_project(
    project_path: &Path,
    page_jsons: &[PathBuf],
    target_lang: Option<String>,
) -> Result<()> {
    let pages: Vec<String> = page_jsons
        .iter()
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    let proj = Project {
        version: 1,
        pages,
        target_lang,
        ocr_engine: None,
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
            raw_text: None,
            mask_path: None,
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
