use anyhow::{Context, Result};
use base64::Engine as _;
use image::RgbImage;
use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::metadata::{Bubble, BubbleStyle, PageEditData};
use crate::model::yolo::Detection;
use crate::typesetting::Typesetter;

/// Transactional patch mirroring `metadata edit` flags.
/// All fields default to empty so `--stdin-patch` can send a subset.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EditPatch {
    #[serde(default)]
    pub set: Vec<String>,
    #[serde(default)]
    pub bbox: Vec<String>,
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub delete: Vec<String>,
    #[serde(default)]
    pub font_family: Vec<String>,
    #[serde(default)]
    pub font_size: Vec<String>,
    #[serde(default)]
    pub text_color: Vec<String>,
    #[serde(default)]
    pub stroke_color: Vec<String>,
    #[serde(default)]
    pub align: Vec<String>,
    #[serde(default)]
    pub edited: Vec<String>,
    #[serde(default)]
    pub bg_color: Vec<String>,
    #[serde(default)]
    pub conf: Vec<String>,
    #[serde(default)]
    pub clear_style: Vec<String>,
    #[serde(default)]
    pub raw_text: Vec<String>,
    #[serde(default)]
    pub replace: Vec<String>,
    #[serde(default)]
    pub apply_style: Option<String>,
    #[serde(default)]
    pub bold: Vec<String>,
    #[serde(default)]
    pub italic: Vec<String>,
    #[serde(default)]
    pub order: Option<String>,
    #[serde(default)]
    pub mask_path: Vec<String>,
}

fn parse_bbox(s: &str) -> Option<[u32; 4]> {
    let parts: Vec<u32> = s.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    if parts.len() == 4 {
        Some([parts[0], parts[1], parts[2], parts[3]])
    } else {
        None
    }
}

fn parse_rgb(s: &str) -> Option<[u8; 3]> {
    let parts: Vec<u8> = s.split(',').filter_map(|v| v.trim().parse().ok()).collect();
    if parts.len() == 3 {
        Some([parts[0], parts[1], parts[2]])
    } else {
        None
    }
}

fn check_bbox_in_bounds(b: [u32; 4], w: u32, h: u32) -> Result<()> {
    if b[0] >= b[2] || b[1] >= b[3] {
        anyhow::bail!("Invalid bbox {:?}: require x1<x2 and y1<y2", b);
    }
    if b[2] > w || b[3] > h {
        anyhow::bail!("Bbox {:?} out of bounds {}x{}", b, w, h);
    }
    Ok(())
}

/// Apply a patch to `data`.
///
/// Returns `(logs, errors)`. In lenient mode (legacy CLI flags) valid ops are
/// applied and errors are collected as warnings. In strict mode (stdin-patch)
/// the caller should operate on a clone and only commit when `errors` is empty.
pub fn apply_patch(data: &mut PageEditData, patch: &EditPatch) -> (Vec<String>, Vec<String>) {
    let mut logs = Vec::new();
    let mut errors = Vec::new();

    for s in &patch.set {
        match s.split_once('=') {
            Some((id, txt)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    b.translated = txt.to_string();
                    b.edited = true;
                    logs.push(format!("Set {} = \"{}\"", id, txt));
                } else {
                    errors.push(format!("Bubble {} not found", id));
                }
            }
            None => errors.push(format!("Invalid --set format (expected ID=text): '{}'", s)),
        }
    }

    for s in &patch.bbox {
        match s.split_once('=') {
            Some((id, coords)) => match parse_bbox(coords) {
                Some(b) => match check_bbox_in_bounds(b, data.width, data.height) {
                    Ok(()) => {
                        if let Some(bb) = data.bubbles.iter_mut().find(|b| b.id == id) {
                            bb.bbox = b;
                            logs.push(format!("Set bbox {} = {:?}", id, b));
                        } else {
                            errors.push(format!("Bubble {} not found", id));
                        }
                    }
                    Err(e) => errors.push(format!("Invalid bbox for {}: {}", id, e)),
                },
                None => errors.push(format!(
                    "Invalid bbox format for {}: expected x1,y1,x2,y2",
                    id
                )),
            },
            None => errors.push(format!(
                "Invalid --bbox format (expected ID=x1,y1,x2,y2): '{}'",
                s
            )),
        }
    }

    for s in &patch.add {
        match s.split_once('=') {
            Some((id, rest)) => {
                let (bbox_str, txt) = if let Some(eq_pos) = rest.find('=') {
                    let candidate = &rest[..eq_pos];
                    if candidate.matches(',').count() == 3 {
                        (&rest[..eq_pos], &rest[eq_pos + 1..])
                    } else {
                        (rest, "")
                    }
                } else {
                    (rest, "")
                };
                if data.bubbles.iter().any(|b| b.id == id) {
                    errors.push(format!("Bubble {} already exists, skip add", id));
                    continue;
                }
                match parse_bbox(bbox_str) {
                    Some(b) => match check_bbox_in_bounds(b, data.width, data.height) {
                        Ok(()) => {
                            data.bubbles.push(Bubble {
                                id: id.to_string(),
                                bbox: b,
                                conf: 1.0,
                                translated: txt.to_string(),
                                bg_color: None,
                                style: None,
                                edited: false,
                                raw_text: None,
                                mask_path: None,
                            });
                            logs.push(format!("Added bubble {} bbox {:?} text \"{}\"", id, b, txt));
                        }
                        Err(e) => errors.push(format!("Invalid bbox for add {}: {}", id, e)),
                    },
                    None => {
                        errors.push(format!("Invalid bbox for add {}: expected x1,y1,x2,y2", id))
                    }
                }
            }
            None => errors.push(format!(
                "Invalid --add format: expected ID=x1,y1,x2,y2[=text] got '{}'",
                s
            )),
        }
    }

    for id in &patch.delete {
        let before = data.bubbles.len();
        data.bubbles.retain(|b| b.id != *id);
        if data.bubbles.len() < before {
            logs.push(format!("Deleted bubble {}", id));
        } else {
            errors.push(format!("Bubble {} not found for delete", id));
        }
    }

    for s in &patch.font_family {
        match s.split_once('=') {
            Some((id, font)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    if font.is_empty() {
                        if let Some(st) = b.style.as_mut() {
                            st.font_family = None;
                        }
                        logs.push(format!("Cleared font for {}", id));
                    } else {
                        let st = b.style.get_or_insert_with(BubbleStyle::default);
                        st.font_family = Some(font.to_string());
                        logs.push(format!("Set font {} = {}", id, font));
                    }
                } else {
                    errors.push(format!("Bubble {} not found for font_family", id));
                }
            }
            None => errors.push(format!(
                "Invalid --font-family format (expected ID=Font): '{}'",
                s
            )),
        }
    }

    for s in &patch.font_size {
        match s.split_once('=') {
            Some((id, val)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    if val.is_empty() {
                        if let Some(st) = b.style.as_mut() {
                            st.font_size = None;
                        }
                        logs.push(format!("Cleared font_size for {}", id));
                    } else if let Ok(f) = val.parse::<f32>() {
                        let st = b.style.get_or_insert_with(BubbleStyle::default);
                        st.font_size = Some(f);
                        logs.push(format!("Set font_size {} = {}", id, f));
                    } else {
                        errors.push(format!("Invalid font_size for {}: {}", id, val));
                    }
                } else {
                    errors.push(format!("Bubble {} not found", id));
                }
            }
            None => errors.push(format!(
                "Invalid --font-size format (expected ID=Size): '{}'",
                s
            )),
        }
    }

    for s in &patch.text_color {
        match s.split_once('=') {
            Some((id, val)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    if val.is_empty() {
                        if let Some(st) = b.style.as_mut() {
                            st.text_color = None;
                        }
                        logs.push(format!("Cleared text_color for {}", id));
                    } else if let Some(c) = parse_rgb(val) {
                        let st = b.style.get_or_insert_with(BubbleStyle::default);
                        st.text_color = Some(c);
                        logs.push(format!("Set text_color {} = {:?}", id, c));
                    } else {
                        errors.push(format!("Invalid text_color for {}: expected R,G,B", id));
                    }
                } else {
                    errors.push(format!("Bubble {} not found", id));
                }
            }
            None => errors.push(format!("Invalid --text-color format: '{}'", s)),
        }
    }

    for s in &patch.stroke_color {
        match s.split_once('=') {
            Some((id, val)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    if val.is_empty() {
                        if let Some(st) = b.style.as_mut() {
                            st.stroke_color = None;
                        }
                        logs.push(format!("Cleared stroke_color for {}", id));
                    } else if let Some(c) = parse_rgb(val) {
                        let st = b.style.get_or_insert_with(BubbleStyle::default);
                        st.stroke_color = Some(c);
                        logs.push(format!("Set stroke_color {} = {:?}", id, c));
                    } else {
                        errors.push(format!("Invalid stroke_color for {}: expected R,G,B", id));
                    }
                } else {
                    errors.push(format!("Bubble {} not found", id));
                }
            }
            None => errors.push(format!("Invalid --stroke-color format: '{}'", s)),
        }
    }

    for s in &patch.align {
        match s.split_once('=') {
            Some((id, val)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    if val.is_empty() {
                        if let Some(st) = b.style.as_mut() {
                            st.align = None;
                        }
                        logs.push(format!("Cleared align for {}", id));
                    } else {
                        let st = b.style.get_or_insert_with(BubbleStyle::default);
                        st.align = Some(val.to_string());
                        logs.push(format!("Set align {} = {}", id, val));
                    }
                } else {
                    errors.push(format!("Bubble {} not found", id));
                }
            }
            None => errors.push(format!("Invalid --align format: '{}'", s)),
        }
    }

    for s in &patch.edited {
        match s.split_once('=') {
            Some((id, val)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    match val.parse::<bool>() {
                        Ok(v) => {
                            b.edited = v;
                            logs.push(format!("Set edited {} = {}", id, v));
                        }
                        Err(_) => {
                            errors.push(format!("Invalid edited for {}: expected true/false", id))
                        }
                    }
                } else {
                    errors.push(format!("Bubble {} not found", id));
                }
            }
            None => errors.push(format!("Invalid --edited format: '{}'", s)),
        }
    }

    for s in &patch.bg_color {
        match s.split_once('=') {
            Some((id, val)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    if val.is_empty() {
                        b.bg_color = None;
                        logs.push(format!("Cleared bg_color for {}", id));
                    } else if let Some(c) = parse_rgb(val) {
                        b.bg_color = Some(c);
                        logs.push(format!("Set bg_color {} = {:?}", id, c));
                    } else {
                        errors.push(format!("Invalid bg_color for {}: expected R,G,B", id));
                    }
                } else {
                    errors.push(format!("Bubble {} not found", id));
                }
            }
            None => errors.push(format!("Invalid --bg-color format: '{}'", s)),
        }
    }

    for s in &patch.conf {
        match s.split_once('=') {
            Some((id, val)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    match val.parse::<f32>() {
                        Ok(v) => {
                            b.conf = v.clamp(0.0, 1.0);
                            logs.push(format!("Set conf {} = {}", id, b.conf));
                        }
                        Err(_) => errors.push(format!("Invalid conf for {}: {}", id, val)),
                    }
                } else {
                    errors.push(format!("Bubble {} not found", id));
                }
            }
            None => errors.push(format!("Invalid --conf format: '{}'", s)),
        }
    }

    for id in &patch.clear_style {
        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == *id) {
            b.style = None;
            logs.push(format!("Cleared style for {}", id));
        } else {
            errors.push(format!("Bubble {} not found for clear_style", id));
        }
    }

    for s in &patch.raw_text {
        match s.split_once('=') {
            Some((id, txt)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    if txt.is_empty() {
                        b.raw_text = None;
                        logs.push(format!("Cleared raw_text for {}", id));
                    } else {
                        b.raw_text = Some(txt.to_string());
                        logs.push(format!("Set raw_text {} = \"{}\"", id, txt));
                    }
                } else {
                    errors.push(format!("Bubble {} not found for raw_text", id));
                }
            }
            None => errors.push(format!("Invalid --raw-text format: '{}'", s)),
        }
    }

    for rep in &patch.replace {
        match rep.split_once('=') {
            Some((old, new)) => {
                let mut cnt = 0;
                for b in data.bubbles.iter_mut() {
                    if b.translated.contains(old) {
                        b.translated = b.translated.replace(old, new);
                        b.edited = true;
                        cnt += 1;
                    }
                }
                logs.push(format!(
                    "Replace \"{}\" -> \"{}\" in {} bubbles",
                    old, new, cnt
                ));
            }
            None => errors.push(format!(
                "Invalid --replace format: expected Old=New got '{}'",
                rep
            )),
        }
    }

    if let Some(spec) = &patch.apply_style {
        let pairs: Vec<(&str, &str)> = spec.split(',').filter_map(|p| p.split_once('=')).collect();
        if pairs.is_empty() && !spec.trim().is_empty() {
            errors.push(format!("Invalid --apply-style spec: '{}'", spec));
        } else {
            let mut cnt = 0;
            for b in data.bubbles.iter_mut() {
                let st = b.style.get_or_insert_with(BubbleStyle::default);
                for (k, v) in &pairs {
                    match k.trim().to_lowercase().as_str() {
                        "align" => {
                            if v.is_empty() {
                                st.align = None
                            } else {
                                st.align = Some(v.to_string())
                            }
                        }
                        "font_size" | "fontsize" | "size" => {
                            if v.is_empty() {
                                st.font_size = None
                            } else if let Ok(f) = v.parse::<f32>() {
                                st.font_size = Some(f)
                            }
                        }
                        "font_family" | "font" => {
                            if v.is_empty() {
                                st.font_family = None
                            } else {
                                st.font_family = Some(v.to_string())
                            }
                        }
                        "text_color" | "tc" => {
                            if v.is_empty() {
                                st.text_color = None
                            } else if let Some(c) = parse_rgb(v) {
                                st.text_color = Some(c)
                            }
                        }
                        "stroke_color" | "sc" => {
                            if v.is_empty() {
                                st.stroke_color = None
                            } else if let Some(c) = parse_rgb(v) {
                                st.stroke_color = Some(c)
                            }
                        }
                        "bold" | "is_bold" => {
                            if v.is_empty() {
                                st.is_bold = None
                            } else if let Ok(bv) = v.parse::<bool>() {
                                st.is_bold = Some(bv)
                            }
                        }
                        "italic" | "is_italic" => {
                            if v.is_empty() {
                                st.is_italic = None
                            } else if let Ok(bv) = v.parse::<bool>() {
                                st.is_italic = Some(bv)
                            }
                        }
                        _ => {}
                    }
                }
                cnt += 1;
            }
            logs.push(format!("Apply style to {} bubbles: {}", cnt, spec));
        }
    }

    for s in &patch.bold {
        match s.split_once('=') {
            Some((id, val)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    if val.is_empty() {
                        if let Some(st) = b.style.as_mut() {
                            st.is_bold = None
                        }
                        logs.push(format!("Cleared bold for {}", id));
                    } else if let Ok(v) = val.parse::<bool>() {
                        let st = b.style.get_or_insert_with(BubbleStyle::default);
                        st.is_bold = Some(v);
                        logs.push(format!("Set bold {} = {}", id, v));
                    } else {
                        errors.push(format!("Invalid bold for {}: {}", id, val));
                    }
                } else {
                    errors.push(format!("Bubble {} not found", id));
                }
            }
            None => errors.push(format!("Invalid --bold format: '{}'", s)),
        }
    }

    for s in &patch.italic {
        match s.split_once('=') {
            Some((id, val)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    if val.is_empty() {
                        if let Some(st) = b.style.as_mut() {
                            st.is_italic = None
                        }
                        logs.push(format!("Cleared italic for {}", id));
                    } else if let Ok(v) = val.parse::<bool>() {
                        let st = b.style.get_or_insert_with(BubbleStyle::default);
                        st.is_italic = Some(v);
                        logs.push(format!("Set italic {} = {}", id, v));
                    } else {
                        errors.push(format!("Invalid italic for {}: {}", id, val));
                    }
                } else {
                    errors.push(format!("Bubble {} not found", id));
                }
            }
            None => errors.push(format!("Invalid --italic format: '{}'", s)),
        }
    }

    if let Some(order_spec) = &patch.order {
        let ids: Vec<String> = order_spec
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let known: std::collections::HashSet<&str> =
            data.bubbles.iter().map(|b| b.id.as_str()).collect();
        let mut bad = Vec::new();
        for id in &ids {
            if !known.contains(id.as_str()) {
                bad.push(id.clone());
            }
        }
        if !bad.is_empty() {
            errors.push(format!("Unknown order ids: {}", bad.join(",")));
        } else {
            data.order = Some(ids);
            logs.push("Set reading order".to_string());
        }
    }

    for s in &patch.mask_path {
        match s.split_once('=') {
            Some((id, val)) => {
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    if val.is_empty() {
                        b.mask_path = None;
                        logs.push(format!("Cleared mask_path for {}", id));
                    } else {
                        b.mask_path = Some(val.to_string());
                        logs.push(format!("Set mask_path {} = {}", id, val));
                    }
                } else {
                    errors.push(format!("Bubble {} not found for mask_path", id));
                }
            }
            None => errors.push(format!("Invalid --mask-path format: '{}'", s)),
        }
    }

    (logs, errors)
}

fn resolve_font_bytes(name: &str) -> Option<Vec<u8>> {
    if crate::font::FontRegistry::resolve(name).is_ok() {
        let list = crate::font::FontRegistry::list();
        if let Some(info) = list.iter().find(|f| f.name == name)
            && let Ok(b) = std::fs::read(&info.path)
        {
            return Some(b);
        }
        // Embedded fallback still resolves; load embedded bytes.
        if name.eq_ignore_ascii_case("Komika Axis") || name.eq_ignore_ascii_case("komika") {
            return Some(include_bytes!("../fonts/Komika Axis.ttf").to_vec());
        }
        if name.eq_ignore_ascii_case("KosugiMaru") || name.eq_ignore_ascii_case("kosugi") {
            return Some(include_bytes!("../fonts/KosugiMaru.ttf").to_vec());
        }
        return None;
    }
    let p = Path::new(name);
    if p.exists() {
        return std::fs::read(p).ok();
    }
    None
}

/// Persistent editor session holding fonts (and optionally YOLO) in memory.
///
/// Construct once per GUI session to avoid reloading fonts/YOLO per command.
/// For Tauri, wrap in `std::sync::Mutex` and share via managed state; all
/// methods are sync and return owned data or base64 PNG (no stdout/exit).
pub struct EditorSession {
    font_bytes: Vec<u8>,
    cjk_bytes: Option<Vec<u8>>,
    yolo: Option<crate::model::yolo::YoloModel>,
}

impl EditorSession {
    pub fn new(font_bytes: Vec<u8>, cjk_bytes: Option<Vec<u8>>) -> Result<Self> {
        // Validate fonts early so GUI fails fast.
        Typesetter::new(&font_bytes, cjk_bytes.as_deref())?;
        Ok(Self {
            font_bytes,
            cjk_bytes,
            yolo: None,
        })
    }

    /// Opens a session with the YOLO bubble detector loaded once.
    /// Use this for GUI sessions that also run detection/export.
    pub fn open_model(
        model_path: &Path,
        font_bytes: Vec<u8>,
        cjk_bytes: Option<Vec<u8>>,
    ) -> Result<Self> {
        let mut session = Self::new(font_bytes, cjk_bytes)?;
        session.yolo = Some(crate::model::yolo::YoloModel::new(model_path)?);
        Ok(session)
    }

    pub fn has_detector(&self) -> bool {
        self.yolo.is_some()
    }

    /// Runs bubble detection with the session-persisted YOLO model.
    pub fn detect_bubbles(&mut self, img: &image::DynamicImage) -> Result<Vec<Detection>> {
        let yolo = self
            .yolo
            .as_mut()
            .context("No YOLO model loaded: use EditorSession::open_model")?;
        yolo.detect_bubbles(img)
    }

    /// Detects bubbles on an image file and builds an empty `PageEditData`
    /// (no LLM): the cheap export path for editors.
    pub fn export_page(&mut self, img_path: &Path) -> Result<PageEditData> {
        let img =
            image::open(img_path).with_context(|| format!("Failed to open {:?}", img_path))?;
        let (w, h) = (img.width(), img.height());
        let dets = self.detect_bubbles(&img)?;
        let bubbles: Vec<Bubble> = dets
            .iter()
            .enumerate()
            .map(|(i, d)| Bubble::detected((i + 1).to_string(), [d.x1, d.y1, d.x2, d.y2], d.conf))
            .collect();
        let mut data = PageEditData::new(
            img_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            w,
            h,
            "English".to_string(),
            "classic".to_string(),
            bubbles,
        );
        let boxes: Vec<[u32; 4]> = data.bubbles.iter().map(|b| b.bbox).collect();
        let ord = crate::preparer::detect_reading_order(&boxes, crate::preparer::ReadingMode::R2L);
        if ord.len() == data.bubbles.len() {
            data.order = Some(ord.iter().map(|&i| data.bubbles[i].id.clone()).collect());
        }
        Ok(data)
    }

    pub fn load_page(&self, path: &Path) -> Result<PageEditData> {
        crate::metadata::load_page_metadata(path)
    }

    /// Render a full page: inpaint translated bubbles then typeset in reading order.
    pub fn render_page(&self, img: &RgbImage, data: &PageEditData) -> Result<RgbImage> {
        let mut rgb = img.clone();
        let ordered = data.ordered_bubbles();
        let regions: Vec<crate::inpaint::MaskedRegion> = ordered
            .iter()
            .filter(|b| b.translated.to_uppercase() != "SKIP" && !b.translated.trim().is_empty())
            .map(|b| {
                let det = Detection {
                    x1: b.bbox[0],
                    y1: b.bbox[1],
                    x2: b.bbox[2],
                    y2: b.bbox[3],
                    conf: b.conf,
                };
                let mask = b.mask_path.as_deref().map(|mp| {
                    let w = det.width().max(1);
                    let h = det.height().max(1);
                    crate::inpaint::load_mask_for(Path::new(mp), w, h)
                        .with_context(|| format!("Bubble {}: invalid mask {:?}", b.id, mp))
                });
                // Fail fast on unreadable/mismatched masks (GUI brush feedback).
                let mask = mask.transpose()?;
                Ok(crate::inpaint::MaskedRegion { det, mask })
            })
            .collect::<Result<_>>()?;
        if !regions.is_empty() {
            crate::inpaint::inpaint_regions(&mut rgb, &regions)?;
        }
        let global = Typesetter::new(&self.font_bytes, self.cjk_bytes.as_deref())?;
        for b in ordered {
            if b.translated.to_uppercase() == "SKIP" || b.translated.trim().is_empty() {
                continue;
            }
            let det = Detection {
                x1: b.bbox[0],
                y1: b.bbox[1],
                x2: b.bbox[2],
                y2: b.bbox[3],
                conf: b.conf,
            };
            let fam = b.style.as_ref().and_then(|s| s.font_family.as_deref());
            if let Some(fname) = fam
                && let Some(bytes) = resolve_font_bytes(fname)
                && let Ok(ts) = Typesetter::new(&bytes, self.cjk_bytes.as_deref())
            {
                ts.render_bubble_text_with_style(
                    &mut rgb,
                    &det,
                    &b.translated,
                    Some(&data.target_lang),
                    None,
                    b.style.as_ref(),
                );
                continue;
            }
            global.render_bubble_text_with_style(
                &mut rgb,
                &det,
                &b.translated,
                Some(&data.target_lang),
                None,
                b.style.as_ref(),
            );
        }
        Ok(rgb)
    }

    /// Render a single bubble (inpaint + typeset) for live preview.
    pub fn preview_bubble(
        &self,
        img: &RgbImage,
        data: &PageEditData,
        id: &str,
        text_override: Option<&str>,
        style_override: Option<&BubbleStyle>,
    ) -> Result<RgbImage> {
        let bubble = data
            .bubbles
            .iter()
            .find(|b| b.id == id)
            .with_context(|| format!("Bubble {} not found", id))?;
        let text = text_override.unwrap_or(&bubble.translated);
        let mut rgb = img.clone();
        let det = Detection {
            x1: bubble.bbox[0],
            y1: bubble.bbox[1],
            x2: bubble.bbox[2],
            y2: bubble.bbox[3],
            conf: bubble.conf,
        };
        let mask = bubble
            .mask_path
            .as_deref()
            .map(|mp| {
                crate::inpaint::load_mask_for(
                    Path::new(mp),
                    det.width().max(1),
                    det.height().max(1),
                )
                .with_context(|| format!("Bubble {}: invalid mask {:?}", id, mp))
            })
            .transpose()?;
        crate::inpaint::inpaint_regions(
            &mut rgb,
            &[crate::inpaint::MaskedRegion {
                det: det.clone(),
                mask,
            }],
        )?;
        let style_owned;
        let style_ref = if let Some(ov) = style_override {
            style_owned = ov.clone();
            Some(&style_owned)
        } else {
            bubble.style.as_ref()
        };
        if let Some(fname) = style_ref.and_then(|s| s.font_family.as_deref())
            && let Some(bytes) = resolve_font_bytes(fname)
            && let Ok(ts) = Typesetter::new(&bytes, self.cjk_bytes.as_deref())
        {
            ts.render_bubble_text_with_style(
                &mut rgb,
                &det,
                text,
                Some(&data.target_lang),
                None,
                style_ref,
            );
            return Ok(rgb);
        }
        let global = Typesetter::new(&self.font_bytes, self.cjk_bytes.as_deref())?;
        global.render_bubble_text_with_style(
            &mut rgb,
            &det,
            text,
            Some(&data.target_lang),
            None,
            style_ref,
        );
        Ok(rgb)
    }
}

/// Encode an image as a single-line base64 PNG (for GUI live preview).
pub fn png_base64(img: &RgbImage) -> Result<String> {
    let dynimg = image::DynamicImage::ImageRgb8(img.clone());
    let mut buf = std::io::Cursor::new(Vec::new());
    dynimg
        .write_to(&mut buf, image::ImageFormat::Png)
        .context("Failed to encode PNG")?;
    Ok(base64::engine::general_purpose::STANDARD.encode(buf.into_inner()))
}

/// Downscale so the longest side is at most `max_side` (for `--thumb`).
pub fn thumbnail(img: &RgbImage, max_side: u32) -> RgbImage {
    if max_side == 0 {
        return img.clone();
    }
    let (w, h) = (img.width(), img.height());
    let longest = w.max(h);
    if longest <= max_side {
        return img.clone();
    }
    let scale = max_side as f32 / longest as f32;
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    image::imageops::resize(img, nw, nh, image::imageops::FilterType::Triangle)
}

/// Hash bubbles for watch dirty-detection.
pub fn bubbles_hash(data: &PageEditData) -> String {
    let v = serde_json::to_value(&data.bubbles).unwrap_or(serde_json::Value::Null);
    let s = serde_json::to_string(&v).unwrap_or_default();
    blake3::hash(s.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_data() -> PageEditData {
        PageEditData::new(
            "p.jpg".to_string(),
            1000,
            1000,
            "Indonesian".to_string(),
            "classic".to_string(),
            vec![Bubble {
                id: "1".to_string(),
                bbox: [10, 10, 100, 100],
                conf: 0.9,
                translated: "Halo".to_string(),
                bg_color: None,
                style: None,
                edited: false,
                raw_text: None,
                mask_path: None,
            }],
        )
    }

    #[test]
    fn patch_set_and_bbox_lenient() {
        let mut d = sample_data();
        let p = EditPatch {
            set: vec!["1=Hai".to_string()],
            bbox: vec!["1=0,0,50,50".to_string()],
            ..Default::default()
        };
        let (logs, errors) = apply_patch(&mut d, &p);
        assert!(errors.is_empty());
        assert_eq!(logs.len(), 2);
        assert_eq!(d.bubbles[0].translated, "Hai");
        assert_eq!(d.bubbles[0].bbox, [0, 0, 50, 50]);
    }

    #[test]
    fn patch_strict_collects_errors_without_commit_by_caller() {
        let d = sample_data();
        let mut clone = d.clone();
        let p = EditPatch {
            set: vec!["99=Missing".to_string()],
            bbox: vec!["1=5000,0,10,10".to_string()],
            ..Default::default()
        };
        let (_logs, errors) = apply_patch(&mut clone, &p);
        assert!(!errors.is_empty());
        // Caller discards clone on error: original untouched.
        assert_eq!(d.bubbles[0].translated, "Halo");
    }

    #[test]
    fn order_rejects_unknown_ids() {
        let mut d = sample_data();
        let p = EditPatch {
            order: Some("1,99".to_string()),
            ..Default::default()
        };
        let (_logs, errors) = apply_patch(&mut d, &p);
        assert!(!errors.is_empty());
        assert!(d.order.is_none());
    }

    #[test]
    fn lock_allows_concurrent_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.kedit.json");
        let v1 = serde_json::json!({"a":1});
        let v2 = serde_json::json!({"a":2});
        let p1 = path.clone();
        let p2 = path.clone();
        let h1 = std::thread::spawn(move || {
            for _ in 0..5 {
                crate::metadata::save_page_metadata(&p1, &sample_data()).unwrap();
                let _ = &v1;
            }
        });
        let h2 = std::thread::spawn(move || {
            for _ in 0..5 {
                crate::metadata::save_page_metadata(&p2, &sample_data()).unwrap();
                let _ = &v2;
            }
        });
        h1.join().unwrap();
        h2.join().unwrap();
        let back = crate::metadata::load_page_metadata(&path).unwrap();
        assert_eq!(back.bubbles.len(), 1);
    }

    #[test]
    fn editor_session_is_send_for_tauri_state() {
        fn assert_send<T: Send>() {}
        assert_send::<EditorSession>();
        assert_send::<crate::model::yolo::YoloModel>();
    }
}
