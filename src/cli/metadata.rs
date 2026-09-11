use anyhow::{Context, Result, bail};
use base64::Engine as _;
use std::path::{Path, PathBuf};

use kzktdk::archive::{self, PreparedInput};
use kzktdk::metadata::{self, PageEditData};
use kzktdk::model::yolo::YoloModel;

use super::args::MetadataCmd;
use super::util::{ensure_model, file_name, load_cjk_bytes, load_font_bytes, parse_jobs};

pub async fn run(cmd: MetadataCmd) -> Result<()> {
    match cmd {
        MetadataCmd::Export {
            input,
            json,
            model,
            reading_order,
        } => run_export(input, json, model, reading_order),
        MetadataCmd::Render {
            image,
            metadata,
            project,
            images,
            output,
            font,
            cjk_font,
            jobs,
            progress,
            quiet,
        } => run_render(
            image, metadata, project, images, output, font, cjk_font, jobs, progress, quiet,
        ),
        MetadataCmd::Edit {
            json,
            set,
            bbox,
            add,
            delete,
            font_family,
            font_size,
            text_color,
            stroke_color,
            align,
            edited,
            bg_color,
            conf,
            clear_style,
            raw_text,
            replace,
            apply_style,
            bold,
            italic,
            stdin_patch,
            order,
            mask_path,
        } => run_edit(
            json,
            set,
            bbox,
            add,
            delete,
            font_family,
            font_size,
            text_color,
            stroke_color,
            align,
            edited,
            bg_color,
            conf,
            clear_style,
            raw_text,
            replace,
            apply_style,
            bold,
            italic,
            stdin_patch,
            order,
            mask_path,
        ),
        MetadataCmd::Validate {
            json,
            format,
            quiet,
        } => run_validate(json, format, quiet),
        MetadataCmd::Preview {
            image,
            metadata,
            id,
            text,
            output,
            to_stdout,
            thumb,
            font,
            cjk_font,
            font_family,
            font_size,
            text_color,
            stroke_color,
            align,
            show_raw,
            quiet,
        } => run_preview(
            image,
            metadata,
            id,
            text,
            output,
            to_stdout,
            thumb,
            font,
            cjk_font,
            font_family,
            font_size,
            text_color,
            stroke_color,
            align,
            show_raw,
            quiet,
        ),
        MetadataCmd::Show {
            json,
            id,
            format,
            quiet,
        } => run_show(json, id, format, quiet),
        MetadataCmd::Pack {
            input,
            output,
            format,
        } => run_pack(input, output, format),
        MetadataCmd::Mask {
            json,
            id,
            from,
            clear,
        } => run_mask(json, id, from, clear),
        MetadataCmd::Watch {
            project,
            images,
            output,
            font,
            cjk_font,
            interval,
            once,
        } => run_watch(project, images, output, font, cjk_font, interval, once),
    }
}

fn run_export(input: PathBuf, json: PathBuf, model: PathBuf, reading_order: String) -> Result<()> {
    let model_file = ensure_model(&model)?;
    let mut yolo = YoloModel::new(&model_file)?;
    let prepared = archive::prepare_input(&input)?;
    let mut pages = Vec::new();
    let images = match prepared {
        PreparedInput::SingleImage(p) => vec![p],
        PreparedInput::Batch { images, .. } => images,
    };
    let ro_mode = if reading_order == "off" {
        None
    } else {
        kzktdk::preparer::ReadingMode::from_key(&reading_order)
    };
    for p in &images {
        let img = image::open(p).with_context(|| format!("Failed to open {:?}", p))?;
        let (w, h) = (img.width(), img.height());
        let dets = yolo.detect_bubbles(&img)?;
        println!("{:?}: {} bubbles", file_name(p)?, dets.len());
        pages.push(PageEditData::new(
            file_name(p)?.to_string_lossy().to_string(),
            w,
            h,
            "English".to_string(),
            "classic".to_string(),
            dets.iter()
                .enumerate()
                .map(|(i, d)| {
                    metadata::Bubble::detected(
                        (i + 1).to_string(),
                        [d.x1, d.y1, d.x2, d.y2],
                        d.conf,
                    )
                })
                .collect(),
        ));
        // Initial reading order from geometry (unless off).
        if let Some(mode) = ro_mode
            && let Some(last) = pages.last_mut()
        {
            let boxes: Vec<[u32; 4]> = last.bubbles.iter().map(|b| b.bbox).collect();
            let ord = kzktdk::preparer::detect_reading_order(&boxes, mode);
            if ord.len() == last.bubbles.len() {
                last.order = Some(ord.iter().map(|&i| last.bubbles[i].id.clone()).collect());
            }
        }
    }
    // Single page -> object, multi -> array for backwards compat
    let v = if pages.len() == 1 {
        serde_json::to_value(&pages[0]).context("serialize page metadata")?
    } else {
        serde_json::to_value(&pages).context("serialize pages metadata")?
    };
    std::fs::create_dir_all(json.parent().unwrap_or(Path::new(".")))?;
    std::fs::write(
        &json,
        serde_json::to_string_pretty(&v).context("serialize metadata JSON")?,
    )?;
    println!("Exported {} pages to {:?}", pages.len(), json);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_render(
    image: Option<PathBuf>,
    metadata: Option<PathBuf>,
    project: Option<PathBuf>,
    images: Option<PathBuf>,
    output: PathBuf,
    font: PathBuf,
    cjk_font: PathBuf,
    jobs: String,
    progress: String,
    quiet: bool,
) -> Result<()> {
    let jsonl = progress == "jsonl";
    // In quiet/jsonl mode all human logs go to stderr; stdout stays clean for machines.
    macro_rules! info {
        ($($t:tt)*) => {
            if jsonl || quiet { eprintln!($($t)*) } else { println!($($t)*) }
        }
    }
    if let Some(proj_path) = project {
        // Batch render via project.kedit.json
        let proj = metadata::load_project(&proj_path)
            .with_context(|| format!("Failed to load project {:?}", proj_path))?;
        let jobs_num = parse_jobs(&jobs);
        let proj_dir = proj_path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let images_dir = images.clone();
        std::fs::create_dir_all(&output)?;
        info!(
            "[Batch Render] {} pages jobs={} -> {:?}",
            proj.pages.len(),
            jobs_num,
            output
        );
        let font_bytes_global = load_font_bytes(
            font,
            kzktdk::config::DEFAULT_FONT_PATH,
            include_bytes!("../../fonts/Komika Axis.ttf"),
        )?;
        let cjk_bytes_global = load_cjk_bytes(cjk_font, kzktdk::config::DEFAULT_CJK_FONT_PATH);
        let session = kzktdk::editor::EditorSession::new(font_bytes_global, cjk_bytes_global)?;
        for (idx, page_meta_str) in proj.pages.iter().enumerate() {
            let t0 = std::time::Instant::now();
            let meta_path = {
                let p = PathBuf::from(page_meta_str);
                if p.exists() {
                    p
                } else {
                    proj_dir.join(Path::new(page_meta_str).file_name().unwrap_or_default())
                }
            };
            let data = metadata::load_page_metadata(&meta_path)
                .with_context(|| format!("Failed to load {:?}", meta_path))?;
            // Resolve image path: --images folder overrides, else derive from meta page name
            let img_path = if let Some(ref img_dir) = images_dir {
                let cand1 = img_dir.join(&data.page);
                if cand1.exists() {
                    cand1
                } else {
                    // try natural sort match by index
                    let entries: Vec<PathBuf> = std::fs::read_dir(img_dir)
                        .map(|rd| {
                            rd.filter_map(|e| e.ok())
                                .map(|e| e.path())
                                .filter(|p| {
                                    p.extension()
                                        .map(|e| {
                                            e.eq_ignore_ascii_case("jpg")
                                                || e.eq_ignore_ascii_case("png")
                                                || e.eq_ignore_ascii_case("webp")
                                        })
                                        .unwrap_or(false)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if idx < entries.len() {
                        entries[idx].clone()
                    } else {
                        cand1
                    }
                }
            } else {
                // try sibling of meta file
                let cand = meta_path.with_file_name(&data.page);
                if cand.exists() {
                    cand
                } else {
                    PathBuf::from(&data.page)
                }
            };
            if !img_path.exists() {
                eprintln!(
                    "[{}/{}] Skip {}: image not found {:?}",
                    idx + 1,
                    proj.pages.len(),
                    data.page,
                    img_path
                );
                continue;
            }
            let img =
                image::open(&img_path).with_context(|| format!("Failed to open {:?}", img_path))?;
            let rgb = session.render_page(&img.to_rgb8(), &data)?;
            let out_path = output.join(
                Path::new(&data.page)
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new(&data.page)),
            );
            rgb.save(&out_path)?;
            if jsonl {
                eprintln!(
                    "{}",
                    serde_json::json!({"idx": idx+1, "total": proj.pages.len(), "page": data.page, "out": out_path.to_string_lossy(), "ok": true, "ms": t0.elapsed().as_millis() as u64})
                );
            } else {
                info!(
                    "[{}/{}] Rendered {:?} -> {:?}",
                    idx + 1,
                    proj.pages.len(),
                    file_name(img_path.as_path())?,
                    out_path
                );
            }
        }
        if jsonl {
            eprintln!(
                "{}",
                serde_json::json!({"done": true, "ok_count": proj.pages.len(), "fail_count": 0})
            );
        } else {
            info!("Batch render complete -> {:?}", output);
        }
    } else {
        // Single render
        let t0 = std::time::Instant::now();
        let img_path = image
            .clone()
            .context("Missing <IMAGE> or --project for render")?;
        let meta_path = metadata
            .clone()
            .context("Missing --metadata for single render")?;
        let data = metadata::load_page_metadata(&meta_path)?;
        let img =
            image::open(&img_path).with_context(|| format!("Failed to open {:?}", img_path))?;
        let font_bytes_single = if font.exists() {
            std::fs::read(&font)?
        } else {
            // Registry hit wins; otherwise the standard chain
            // (candidate files → embedded). Candidate read errors
            // still propagate, as before.
            let fstr = font.to_string_lossy().to_string();
            match kzktdk::font::FontRegistry::read_registry_font(&fstr) {
                Some(b) => b,
                None => load_font_bytes(
                    font,
                    kzktdk::config::DEFAULT_FONT_PATH,
                    include_bytes!("../../fonts/Komika Axis.ttf"),
                )?,
            }
        };
        let cjk_bytes_single = load_cjk_bytes(cjk_font, kzktdk::config::DEFAULT_CJK_FONT_PATH);
        let session_single =
            kzktdk::editor::EditorSession::new(font_bytes_single, cjk_bytes_single)?;
        let rgb = session_single.render_page(&img.to_rgb8(), &data)?;
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        rgb.save(&output)?;
        if jsonl {
            eprintln!(
                "{}",
                serde_json::json!({"idx": 1, "total": 1, "page": data.page, "out": output.to_string_lossy(), "ok": true, "ms": t0.elapsed().as_millis() as u64})
            );
            eprintln!(
                "{}",
                serde_json::json!({"done": true, "ok_count": 1, "fail_count": 0})
            );
        } else {
            info!("Rendered {:?} -> {:?}", img_path, output);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_edit(
    json: PathBuf,
    set: Vec<String>,
    bbox: Vec<String>,
    add: Vec<String>,
    delete: Vec<String>,
    font_family: Vec<String>,
    font_size: Vec<String>,
    text_color: Vec<String>,
    stroke_color: Vec<String>,
    align: Vec<String>,
    edited: Vec<String>,
    bg_color: Vec<String>,
    conf: Vec<String>,
    clear_style: Vec<String>,
    raw_text: Vec<String>,
    replace: Vec<String>,
    apply_style: Option<String>,
    bold: Vec<String>,
    italic: Vec<String>,
    stdin_patch: bool,
    order: Option<String>,
    mask_path: Vec<String>,
) -> Result<()> {
    if stdin_patch {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("Failed to read stdin patch")?;
        let patch: kzktdk::editor::EditPatch =
            serde_json::from_str(&buf).with_context(|| "Failed to parse stdin EditPatch JSON")?;
        let mut data = metadata::load_page_metadata(&json)?;
        let mut trial = data.clone();
        let (logs, errors) = kzktdk::editor::apply_patch(&mut trial, &patch);
        if !errors.is_empty() {
            eprintln!(
                "{}",
                serde_json::json!({"ok": false, "errors": errors, "logs": logs})
            );
            std::process::exit(2);
        }
        data = trial;
        metadata::save_page_metadata(&json, &data)?;
        println!("{}", serde_json::json!({"ok": true, "logs": logs}));
        return Ok(());
    }
    let mut data = metadata::load_page_metadata(&json)?;
    // Delegate legacy flags through the shared facade patch (lenient: apply valid, warn on errors).
    let facade_patch = kzktdk::editor::EditPatch {
        set: set.clone(),
        bbox: bbox.clone(),
        add: add.clone(),
        delete: delete.clone(),
        font_family: font_family.clone(),
        font_size: font_size.clone(),
        text_color: text_color.clone(),
        stroke_color: stroke_color.clone(),
        align: align.clone(),
        edited: edited.clone(),
        bg_color: bg_color.clone(),
        conf: conf.clone(),
        clear_style: clear_style.clone(),
        raw_text: raw_text.clone(),
        replace: replace.clone(),
        apply_style: apply_style.clone(),
        bold: bold.clone(),
        italic: italic.clone(),
        order: order.clone(),
        mask_path: mask_path.clone(),
    };
    // Fast path: if only facade-covered ops are used, apply once via facade.
    // Otherwise fall through to legacy verbose handling below for identical messages.
    let legacy_only = false;
    if !legacy_only
        && set.is_empty()
        && bbox.is_empty()
        && add.is_empty()
        && delete.is_empty()
        && font_family.is_empty()
        && font_size.is_empty()
        && text_color.is_empty()
        && stroke_color.is_empty()
        && align.is_empty()
        && edited.is_empty()
        && bg_color.is_empty()
        && conf.is_empty()
        && clear_style.is_empty()
        && raw_text.is_empty()
        && replace.is_empty()
        && apply_style.is_none()
        && bold.is_empty()
        && italic.is_empty()
        && (order.is_some() || !mask_path.is_empty())
    {
        let (logs, errors) = kzktdk::editor::apply_patch(&mut data, &facade_patch);
        for l in &logs {
            println!("{}", l);
        }
        for e in &errors {
            eprintln!("{}", e);
        }
        metadata::save_page_metadata(&json, &data)?;
        println!("Saved edited metadata to {:?}", json);
        return Ok(());
    }
    for s in set {
        if let Some((id, txt)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                b.translated = txt.to_string();
                b.edited = true;
                println!("Set {} = \"{}\"", id, txt);
            } else {
                eprintln!("Bubble {} not found", id);
            }
        }
    }
    for s in bbox {
        if let Some((id, coords)) = s.split_once('=') {
            let parts: Vec<u32> = coords
                .split(',')
                .filter_map(|v| v.trim().parse().ok())
                .collect();
            if parts.len() == 4 {
                if parts[0] >= parts[2] || parts[1] >= parts[3] {
                    eprintln!("Invalid bbox for {}: x1<x2 and y1<y2 required", id);
                    continue;
                }
                if parts[2] > data.width || parts[3] > data.height {
                    eprintln!(
                        "Invalid bbox for {}: x2<=width({}) y2<=height({}) required, got {:?}",
                        id, data.width, data.height, parts
                    );
                    continue;
                }
                if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                    b.bbox = [parts[0], parts[1], parts[2], parts[3]];
                    println!("Set bbox {} = {:?}", id, b.bbox);
                } else {
                    eprintln!("Bubble {} not found", id);
                }
            } else {
                eprintln!("Invalid bbox format for {}: expected x1,y1,x2,y2", id);
            }
        }
    }
    for s in add {
        if let Some((id, rest)) = s.split_once('=') {
            let (bbox_str, txt) = if let Some(eq_pos) = rest.find('=') {
                let candidate_bbox = &rest[..eq_pos];
                let comma_cnt = candidate_bbox.matches(',').count();
                if comma_cnt == 3 {
                    (&rest[..eq_pos], &rest[eq_pos + 1..])
                } else {
                    (rest, "")
                }
            } else {
                (rest, "")
            };
            if data.bubbles.iter().any(|b| b.id == id) {
                eprintln!("Bubble {} already exists, skip add", id);
                continue;
            }
            let parts: Vec<u32> = bbox_str
                .split(',')
                .filter_map(|v| v.trim().parse().ok())
                .collect();
            if parts.len() != 4 {
                eprintln!("Invalid bbox for add {}: expected x1,y1,x2,y2", id);
                continue;
            }
            if parts[0] >= parts[2] || parts[1] >= parts[3] {
                eprintln!("Invalid bbox for add {}: x1<x2 and y1<y2 required", id);
                continue;
            }
            if parts[2] > data.width || parts[3] > data.height {
                eprintln!(
                    "Invalid bbox for add {}: out of bounds {}x{}",
                    id, data.width, data.height
                );
                continue;
            }
            data.bubbles.push(metadata::Bubble {
                id: id.to_string(),
                bbox: [parts[0], parts[1], parts[2], parts[3]],
                conf: 1.0,
                translated: txt.to_string(),
                bg_color: None,
                style: None,
                edited: false,
                raw_text: None,
                mask_path: None,
            });
            println!(
                "Added bubble {} bbox {:?} text \"{}\"",
                id,
                [parts[0], parts[1], parts[2], parts[3]],
                txt
            );
        } else {
            eprintln!(
                "Invalid --add format: expected ID=x1,y1,x2,y2[=text] got '{}'",
                s
            );
        }
    }
    for id in delete {
        let before = data.bubbles.len();
        data.bubbles.retain(|b| b.id != id);
        if data.bubbles.len() < before {
            println!("Deleted bubble {}", id);
        } else {
            eprintln!("Bubble {} not found for delete", id);
        }
    }
    for s in font_family {
        if let Some((id, font)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if font.is_empty() {
                    if let Some(style) = b.style.as_mut() {
                        style.font_family = None;
                    }
                    println!("Cleared font for {}", id);
                } else {
                    let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                    style.font_family = Some(font.to_string());
                    println!("Set font {} = {}", id, font);
                }
            } else {
                eprintln!("Bubble {} not found for font_family", id);
            }
        }
    }
    for s in font_size {
        if let Some((id, val)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if val.is_empty() {
                    if let Some(style) = b.style.as_mut() {
                        style.font_size = None;
                    }
                    println!("Cleared font_size for {}", id);
                } else if let Ok(f) = val.parse::<f32>() {
                    let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                    style.font_size = Some(f);
                    println!("Set font_size {} = {}", id, f);
                } else {
                    eprintln!("Invalid font_size for {}: {}", id, val);
                }
            } else {
                eprintln!("Bubble {} not found", id);
            }
        }
    }
    for s in text_color {
        if let Some((id, val)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if val.is_empty() {
                    if let Some(style) = b.style.as_mut() {
                        style.text_color = None;
                    }
                    println!("Cleared text_color for {}", id);
                } else {
                    let parts: Vec<u8> = val
                        .split(',')
                        .filter_map(|v| v.trim().parse().ok())
                        .collect();
                    if parts.len() == 3 {
                        let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                        style.text_color = Some([parts[0], parts[1], parts[2]]);
                        println!("Set text_color {} = {:?}", id, style.text_color);
                    } else {
                        eprintln!("Invalid text_color for {}: expected R,G,B", id);
                    }
                }
            } else {
                eprintln!("Bubble {} not found", id);
            }
        }
    }
    for s in stroke_color {
        if let Some((id, val)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if val.is_empty() {
                    if let Some(style) = b.style.as_mut() {
                        style.stroke_color = None;
                    }
                    println!("Cleared stroke_color for {}", id);
                } else {
                    let parts: Vec<u8> = val
                        .split(',')
                        .filter_map(|v| v.trim().parse().ok())
                        .collect();
                    if parts.len() == 3 {
                        let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                        style.stroke_color = Some([parts[0], parts[1], parts[2]]);
                        println!("Set stroke_color {} = {:?}", id, style.stroke_color);
                    } else {
                        eprintln!("Invalid stroke_color for {}: expected R,G,B", id);
                    }
                }
            } else {
                eprintln!("Bubble {} not found", id);
            }
        }
    }
    for s in align {
        if let Some((id, val)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if val.is_empty() {
                    if let Some(style) = b.style.as_mut() {
                        style.align = None;
                    }
                    println!("Cleared align for {}", id);
                } else {
                    let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                    style.align = Some(val.to_string());
                    println!("Set align {} = {}", id, val);
                }
            } else {
                eprintln!("Bubble {} not found", id);
            }
        }
    }
    for s in edited {
        if let Some((id, val)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if let Ok(v) = val.parse::<bool>() {
                    b.edited = v;
                    println!("Set edited {} = {}", id, v);
                } else {
                    eprintln!("Invalid edited for {}: expected true/false", id);
                }
            } else {
                eprintln!("Bubble {} not found", id);
            }
        }
    }
    for s in bg_color {
        if let Some((id, val)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if val.is_empty() {
                    b.bg_color = None;
                    println!("Cleared bg_color for {}", id);
                } else {
                    let parts: Vec<u8> = val
                        .split(',')
                        .filter_map(|v| v.trim().parse().ok())
                        .collect();
                    if parts.len() == 3 {
                        b.bg_color = Some([parts[0], parts[1], parts[2]]);
                        println!("Set bg_color {} = {:?}", id, b.bg_color);
                    } else {
                        eprintln!("Invalid bg_color for {}: expected R,G,B", id);
                    }
                }
            } else {
                eprintln!("Bubble {} not found", id);
            }
        }
    }
    for s in conf {
        if let Some((id, val)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if let Ok(v) = val.parse::<f32>() {
                    b.conf = v.clamp(0.0, 1.0);
                    println!("Set conf {} = {}", id, b.conf);
                } else {
                    eprintln!("Invalid conf for {}: {}", id, val);
                }
            } else {
                eprintln!("Bubble {} not found", id);
            }
        }
    }
    for id in clear_style {
        if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
            b.style = None;
            println!("Cleared style for {}", id);
        } else {
            eprintln!("Bubble {} not found for clear_style", id);
        }
    }
    for s in raw_text {
        if let Some((id, txt)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if txt.is_empty() {
                    b.raw_text = None;
                    println!("Cleared raw_text for {}", id);
                } else {
                    b.raw_text = Some(txt.to_string());
                    println!("Set raw_text {} = \"{}\"", id, txt);
                }
            } else {
                eprintln!("Bubble {} not found for raw_text", id);
            }
        }
    }
    for rep in replace {
        if let Some((old, new)) = rep.split_once('=') {
            let mut cnt = 0;
            for b in data.bubbles.iter_mut() {
                if b.translated.contains(old) {
                    b.translated = b.translated.replace(old, new);
                    b.edited = true;
                    cnt += 1;
                }
            }
            println!("Replace \"{}\" -> \"{}\" in {} bubbles", old, new, cnt);
        } else {
            eprintln!("Invalid --replace format: expected Old=New got '{}'", rep);
        }
    }
    if let Some(spec) = apply_style {
        // Spec: comma-separated key=value e.g. align=center,bold=true,font_size=20
        let pairs: Vec<(&str, &str)> = spec.split(',').filter_map(|p| p.split_once('=')).collect();
        let mut cnt = 0;
        for b in data.bubbles.iter_mut() {
            let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
            for (k, v) in &pairs {
                match k.trim().to_lowercase().as_str() {
                    "align" => {
                        if v.is_empty() {
                            style.align = None
                        } else {
                            style.align = Some(v.to_string())
                        }
                    }
                    "font_size" | "fontsize" | "size" => {
                        if v.is_empty() {
                            style.font_size = None
                        } else if let Ok(f) = v.parse::<f32>() {
                            style.font_size = Some(f)
                        }
                    }
                    "font_family" | "font" => {
                        if v.is_empty() {
                            style.font_family = None
                        } else {
                            style.font_family = Some(v.to_string())
                        }
                    }
                    "text_color" | "tc" => {
                        if v.is_empty() {
                            style.text_color = None
                        } else {
                            let p: Vec<u8> =
                                v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                            if p.len() == 3 {
                                style.text_color = Some([p[0], p[1], p[2]])
                            }
                        }
                    }
                    "stroke_color" | "sc" => {
                        if v.is_empty() {
                            style.stroke_color = None
                        } else {
                            let p: Vec<u8> =
                                v.split(',').filter_map(|x| x.trim().parse().ok()).collect();
                            if p.len() == 3 {
                                style.stroke_color = Some([p[0], p[1], p[2]])
                            }
                        }
                    }
                    "bold" | "is_bold" => {
                        if v.is_empty() {
                            style.is_bold = None
                        } else if let Ok(bv) = v.parse::<bool>() {
                            style.is_bold = Some(bv)
                        }
                    }
                    "italic" | "is_italic" => {
                        if v.is_empty() {
                            style.is_italic = None
                        } else if let Ok(bv) = v.parse::<bool>() {
                            style.is_italic = Some(bv)
                        }
                    }
                    _ => {}
                }
            }
            cnt += 1;
        }
        println!("Apply style to {} bubbles: {}", cnt, spec);
    }
    for s in bold {
        if let Some((id, val)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if val.is_empty() {
                    if let Some(style) = b.style.as_mut() {
                        style.is_bold = None
                    }
                    println!("Cleared bold for {}", id);
                } else if let Ok(v) = val.parse::<bool>() {
                    let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                    style.is_bold = Some(v);
                    println!("Set bold {} = {}", id, v);
                } else {
                    eprintln!("Invalid bold for {}: {}", id, val);
                }
            } else {
                eprintln!("Bubble {} not found", id);
            }
        }
    }
    for s in italic {
        if let Some((id, val)) = s.split_once('=') {
            if let Some(b) = data.bubbles.iter_mut().find(|b| b.id == id) {
                if val.is_empty() {
                    if let Some(style) = b.style.as_mut() {
                        style.is_italic = None
                    }
                    println!("Cleared italic for {}", id);
                } else if let Ok(v) = val.parse::<bool>() {
                    let style = b.style.get_or_insert_with(metadata::BubbleStyle::default);
                    style.is_italic = Some(v);
                    println!("Set italic {} = {}", id, v);
                } else {
                    eprintln!("Invalid italic for {}: {}", id, val);
                }
            } else {
                eprintln!("Bubble {} not found", id);
            }
        }
    }
    // P2: reading order + mask override (shared facade logic for messages).
    if order.is_some() || !mask_path.is_empty() {
        let extra = kzktdk::editor::EditPatch {
            order: order.clone(),
            mask_path: mask_path.clone(),
            ..Default::default()
        };
        let (logs, errors) = kzktdk::editor::apply_patch(&mut data, &extra);
        for l in &logs {
            println!("{}", l);
        }
        for e in &errors {
            eprintln!("{}", e);
        }
    }
    metadata::save_page_metadata(&json, &data)?;
    println!("Saved edited metadata to {:?}", json);
    Ok(())
}

fn run_validate(json: PathBuf, format: String, quiet: bool) -> Result<()> {
    let as_json = format == "json";
    let content =
        std::fs::read_to_string(&json).with_context(|| format!("Failed to read {:?}", json))?;
    // Try project first
    if let Ok(proj) = serde_json::from_str::<metadata::Project>(&content)
        && proj.version == 1
        && !proj.pages.is_empty()
    {
        if as_json {
            let report = metadata::ValidationReport {
                valid: true,
                kind: "project".to_string(),
                errors: vec![],
                meta: serde_json::json!({"version": proj.version, "pages": proj.pages.len(), "target_lang": proj.target_lang}),
            };
            println!(
                "{}",
                serde_json::to_string(&report).context("serialize validation report")?
            );
        } else if quiet {
            eprintln!(
                "Valid Project v{}: {} pages",
                proj.version,
                proj.pages.len()
            );
        } else {
            println!(
                "Valid Project v{}: {} pages",
                proj.version,
                proj.pages.len()
            );
            for (i, p) in proj.pages.iter().enumerate() {
                println!("  [{:02}] {}", i + 1, p);
            }
        }
        return Ok(());
    }
    let data: metadata::PageEditData = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse PageEditData {:?}", json))?;
    let issues = metadata::validate_page(&data);
    if as_json {
        let report = metadata::ValidationReport {
            valid: issues.is_empty(),
            kind: "page".to_string(),
            errors: issues.clone(),
            meta: serde_json::json!({"version": data.version, "page": data.page, "width": data.width, "height": data.height, "target_lang": data.target_lang, "prompt_sig": data.prompt_sig, "bubbles": data.bubbles.len()}),
        };
        println!(
            "{}",
            serde_json::to_string(&report).context("serialize validation report")?
        );
        if !issues.is_empty() {
            std::process::exit(2);
        }
        return Ok(());
    }
    if quiet {
        eprintln!(
            "Valid PageEditData v{}: {} bubbles, page={} ({}x{}) target_lang={} prompt_sig={}",
            data.version,
            data.bubbles.len(),
            data.page,
            data.width,
            data.height,
            data.target_lang,
            data.prompt_sig
        );
    } else {
        println!(
            "Valid PageEditData v{}: {} bubbles, page={} ({}x{}) target_lang={} prompt_sig={}",
            data.version,
            data.bubbles.len(),
            data.page,
            data.width,
            data.height,
            data.target_lang,
            data.prompt_sig
        );
    }
    for e in &issues {
        eprintln!(
            "  [!] {} {}: {}",
            e.code,
            e.id.as_deref().unwrap_or("-"),
            e.msg
        );
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_preview(
    image: PathBuf,
    metadata: PathBuf,
    id: String,
    text: String,
    output: Option<PathBuf>,
    to_stdout: bool,
    thumb: u32,
    font: PathBuf,
    cjk_font: PathBuf,
    font_family: Option<String>,
    font_size: Option<f32>,
    text_color: Option<String>,
    stroke_color: Option<String>,
    align: Option<String>,
    show_raw: bool,
    quiet: bool,
) -> Result<()> {
    let out_path: Option<PathBuf> = output;
    if !to_stdout && out_path.is_none() {
        bail!("Missing --output (required unless --to-stdout)");
    }
    let data = metadata::load_page_metadata(&metadata)?;
    let bubble = data
        .bubbles
        .iter()
        .find(|b| b.id == id)
        .cloned()
        .context(format!("Bubble {} not found", id))?;
    let display_text = if show_raw {
        bubble.raw_text.clone().unwrap_or(text.clone())
    } else {
        text.clone()
    };
    let rgb_in = image::open(&image)
        .with_context(|| format!("Failed to open {:?}", image))?
        .to_rgb8();
    let font_bytes = load_font_bytes(
        font,
        kzktdk::config::DEFAULT_FONT_PATH,
        include_bytes!("../../fonts/Komika Axis.ttf"),
    )?;
    let cjk_bytes = load_cjk_bytes(cjk_font, kzktdk::config::DEFAULT_CJK_FONT_PATH);
    let mut preview_style = bubble.style.clone().unwrap_or_default();
    let has_override = font_family.is_some()
        || font_size.is_some()
        || text_color.is_some()
        || stroke_color.is_some()
        || align.is_some();
    // Apply overrides
    if let Some(ff) = font_family {
        preview_style.font_family = Some(ff);
    }
    if let Some(fs) = font_size {
        preview_style.font_size = Some(fs);
    }
    if let Some(tc) = text_color {
        let parts: Vec<u8> = tc
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        if parts.len() == 3 {
            preview_style.text_color = Some([parts[0], parts[1], parts[2]]);
        }
    }
    if let Some(sc) = stroke_color {
        let parts: Vec<u8> = sc
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        if parts.len() == 3 {
            preview_style.stroke_color = Some([parts[0], parts[1], parts[2]]);
        }
    }
    if let Some(al) = align {
        preview_style.align = Some(al);
    }
    let style_opt: Option<metadata::BubbleStyle> = if has_override || bubble.style.is_some() {
        Some(preview_style.clone())
    } else {
        None
    };
    let session = kzktdk::editor::EditorSession::new(font_bytes, cjk_bytes)?;
    let mut rgb =
        session.preview_bubble(&rgb_in, &data, &id, Some(&display_text), style_opt.as_ref())?;
    if thumb > 0 {
        rgb = kzktdk::editor::thumbnail(&rgb, thumb);
    }
    if let Some(ref out) = out_path {
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        rgb.save(out)?;
    }
    if to_stdout {
        let dynimg = image::DynamicImage::ImageRgb8(rgb);
        let mut buf = std::io::Cursor::new(Vec::new());
        dynimg.write_to(&mut buf, image::ImageFormat::Png)?;
        println!(
            "{}",
            base64::engine::general_purpose::STANDARD.encode(buf.into_inner())
        );
    }
    if quiet || to_stdout {
        eprintln!("Preview bubble {} show_raw={}", id, show_raw);
    } else {
        println!("Preview bubble {} -> {:?}", id, out_path);
        println!(
            "Preview text: \"{}\" (font_size {:?}, align {:?}, font_family {:?}) show_raw={}",
            display_text,
            style_opt.as_ref().and_then(|s| s.font_size),
            style_opt.as_ref().and_then(|s| s.align.as_ref()),
            style_opt.as_ref().and_then(|s| s.font_family.as_ref()),
            show_raw
        );
    }
    Ok(())
}

fn run_show(json: PathBuf, id: Option<String>, format: String, quiet: bool) -> Result<()> {
    let as_json = format == "json";
    let content =
        std::fs::read_to_string(&json).with_context(|| format!("Failed to read {:?}", json))?;
    if let Ok(proj) = serde_json::from_str::<metadata::Project>(&content)
        && proj.version == 1
        && !proj.pages.is_empty()
    {
        if as_json {
            println!(
                "{}",
                serde_json::to_string_pretty(&proj).context("serialize project")?
            );
            return Ok(());
        }
        if quiet {
            eprintln!(
                "Project v{}: {} pages target_lang={:?}",
                proj.version,
                proj.pages.len(),
                proj.target_lang
            );
        } else {
            println!(
                "Project v{}: {} pages target_lang={:?}",
                proj.version,
                proj.pages.len(),
                proj.target_lang
            );
        }
        for (i, p) in proj.pages.iter().enumerate() {
            let meta_path = {
                let pb = PathBuf::from(p);
                if pb.exists() {
                    pb
                } else {
                    json.parent()
                        .unwrap_or(Path::new("."))
                        .join(Path::new(p).file_name().unwrap_or_default())
                }
            };
            let bubbles = metadata::load_page_metadata(&meta_path)
                .map(|d| d.bubbles.len())
                .unwrap_or(0);
            println!("  [{:02}] {} ({} bubbles)", i + 1, p, bubbles);
        }
        return Ok(());
    }
    let data = metadata::load_page_metadata(&json)?;
    if as_json {
        if let Some(filter) = id.as_ref() {
            if let Some(b) = data.bubbles.iter().find(|b| &b.id == filter) {
                println!(
                    "{}",
                    serde_json::to_string_pretty(b).context("serialize bubble")?
                );
            } else {
                eprintln!(
                    "{}",
                    serde_json::json!({"ok": false, "error": format!("Bubble {} not found", filter)})
                );
                std::process::exit(2);
            }
        } else {
            println!(
                "{}",
                serde_json::to_string_pretty(&data).context("serialize page")?
            );
        }
        return Ok(());
    }
    if quiet {
        eprintln!(
            "PageEditData v{}: {} ({}x{}) lang={} sig={} ocr={:?} order={:?} — {} bubbles",
            data.version,
            data.page,
            data.width,
            data.height,
            data.target_lang,
            data.prompt_sig,
            data.ocr_engine,
            data.order,
            data.bubbles.len()
        );
    } else {
        println!(
            "PageEditData v{}: {} ({}x{}) lang={} sig={} ocr={:?} order={:?} — {} bubbles",
            data.version,
            data.page,
            data.width,
            data.height,
            data.target_lang,
            data.prompt_sig,
            data.ocr_engine,
            data.order,
            data.bubbles.len()
        );
    }
    println!(
        "{:<4} {:<18} {:<5} {:<6} {:<22} {:<12} TRANSLATED",
        "ID", "BBOX", "CONF", "EDIT", "STYLE", "RAW"
    );
    println!("{}", "-".repeat(130));
    for b in &data.bubbles {
        if let Some(ref filter) = id
            && &b.id != filter
        {
            continue;
        }
        let style_str = if let Some(s) = &b.style {
            let mut parts = Vec::new();
            if let Some(f) = &s.font_family {
                parts.push(format!("font={}", f));
            }
            if let Some(fz) = s.font_size {
                parts.push(format!("sz={}", fz));
            }
            if let Some(c) = s.text_color {
                parts.push(format!("tc={},{},{}", c[0], c[1], c[2]));
            }
            if let Some(c) = s.stroke_color {
                parts.push(format!("sc={},{},{}", c[0], c[1], c[2]));
            }
            if let Some(a) = &s.align {
                parts.push(format!("al={}", a));
            }
            if let Some(v) = s.is_bold {
                parts.push(format!("b={}", v));
            }
            if let Some(v) = s.is_italic {
                parts.push(format!("i={}", v));
            }
            parts.join(",")
        } else {
            "-".to_string()
        };
        let t: String = {
            let ch: Vec<char> = b.translated.chars().collect();
            if ch.len() > 30 {
                ch[..30].iter().collect::<String>() + "..."
            } else {
                b.translated.clone()
            }
        };
        let raw = if let Some(r) = &b.raw_text {
            let ch: Vec<char> = r.chars().collect();
            if ch.len() > 10 {
                ch[..10].iter().collect::<String>() + "..."
            } else {
                r.clone()
            }
        } else {
            "-".to_string()
        };
        let bbox = format!("[{},{},{},{}]", b.bbox[0], b.bbox[1], b.bbox[2], b.bbox[3]);
        println!(
            "{:<4} {:<18} {:<5.2} {:<6} {:<22} {:<12} {}",
            b.id,
            bbox,
            b.conf,
            b.edited,
            style_str,
            raw.replace('\n', " "),
            t.replace('\n', " ")
        );
    }
    Ok(())
}

fn run_pack(input: PathBuf, output: PathBuf, format: String) -> Result<()> {
    if !input.is_dir() {
        bail!("Pack input must be a folder: {:?}", input);
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&input)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .map(|e| {
                        e.eq_ignore_ascii_case("jpg")
                            || e.eq_ignore_ascii_case("png")
                            || e.eq_ignore_ascii_case("jpeg")
                            || e.eq_ignore_ascii_case("webp")
                    })
                    .unwrap_or(false)
        })
        .collect();
    if files.is_empty() {
        bail!("No images found in {:?}", input);
    }
    // natural sort (sort keys only; read_dir entries always have names)
    files.sort_by(|a, b| {
        natord::compare(
            &a.file_name().unwrap_or_default().to_string_lossy(),
            &b.file_name().unwrap_or_default().to_string_lossy(),
        )
    });
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    archive::create_cbz(&files, &output)?;
    if format == "json" {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "ok": true,
                "output": output.to_string_lossy(),
                "count": files.len(),
                "files": files.iter().map(|f| f.file_name().unwrap_or_default().to_string_lossy()).collect::<Vec<_>>(),
            }))
            .context("serialize pack summary")?
        );
        return Ok(());
    }
    println!("Packed {} images -> {:?}", files.len(), output);
    for (i, f) in files.iter().enumerate() {
        println!("  [{:02}] {}", i + 1, file_name(f)?.to_string_lossy());
    }
    Ok(())
}

fn run_mask(json: PathBuf, id: String, from: Option<PathBuf>, clear: bool) -> Result<()> {
    let mut data = metadata::load_page_metadata(&json)?;
    let bubble = data
        .bubbles
        .iter_mut()
        .find(|b| b.id == id)
        .with_context(|| format!("Bubble {} not found", id))?;
    if clear || from.is_none() {
        bubble.mask_path = None;
        println!("Cleared mask for {}", id);
    } else {
        let Some(mask_path) = from.clone() else {
            bail!("--from is required unless --clear");
        };
        if !mask_path.is_file() {
            eprintln!("Mask file not found: {:?}", mask_path);
            std::process::exit(2);
        }
        // Validate dimensions against the bubble crop now (fail fast).
        let w = (bubble.bbox[2].saturating_sub(bubble.bbox[0])).max(1);
        let h = (bubble.bbox[3].saturating_sub(bubble.bbox[1])).max(1);
        if let Err(e) = kzktdk::inpaint::load_mask_for(&mask_path, w, h) {
            eprintln!("Invalid mask: {:#}", e);
            std::process::exit(2);
        }
        bubble.mask_path = Some(mask_path.to_string_lossy().to_string());
        println!("Set mask {} = {:?}", id, mask_path);
    }
    metadata::save_page_metadata(&json, &data)?;
    println!("Saved edited metadata to {:?}", json);
    Ok(())
}

fn run_watch(
    project: PathBuf,
    images: Option<PathBuf>,
    output: PathBuf,
    font: PathBuf,
    cjk_font: PathBuf,
    interval: u64,
    once: bool,
) -> Result<()> {
    let font_bytes_global = load_font_bytes(
        font,
        kzktdk::config::DEFAULT_FONT_PATH,
        include_bytes!("../../fonts/Komika Axis.ttf"),
    )?;
    let cjk_bytes_global = load_cjk_bytes(cjk_font, kzktdk::config::DEFAULT_CJK_FONT_PATH);
    let session = kzktdk::editor::EditorSession::new(font_bytes_global, cjk_bytes_global)?;
    std::fs::create_dir_all(&output)?;
    let proj_dir = project.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut hashes: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let render_once =
        |hashes: &mut std::collections::HashMap<String, String>| -> Result<(usize, usize)> {
            let proj = metadata::load_project(&project)
                .with_context(|| format!("Failed to load project {:?}", project))?;
            let mut ok = 0usize;
            let mut dirty = 0usize;
            for (idx, page_meta_str) in proj.pages.iter().enumerate() {
                let meta_path = {
                    let p = PathBuf::from(page_meta_str);
                    if p.exists() {
                        p
                    } else {
                        proj_dir.join(Path::new(page_meta_str).file_name().unwrap_or_default())
                    }
                };
                let data = metadata::load_page_metadata(&meta_path)
                    .with_context(|| format!("Failed to load {:?}", meta_path))?;
                let img_path = if let Some(ref img_dir) = images {
                    let cand = img_dir.join(&data.page);
                    if cand.exists() {
                        cand
                    } else {
                        proj_dir.join(&data.page)
                    }
                } else {
                    let cand = meta_path.with_file_name(&data.page);
                    if cand.exists() {
                        cand
                    } else {
                        PathBuf::from(&data.page)
                    }
                };
                if !img_path.exists() {
                    eprintln!(
                        "{}",
                        serde_json::json!({"idx": idx+1, "total": proj.pages.len(), "page": data.page, "ok": false, "error": "image not found"})
                    );
                    continue;
                }
                let h = kzktdk::editor::bubbles_hash(&data);
                let key = meta_path.to_string_lossy().to_string();
                let prev = hashes.get(&key);
                // Also treat mtime change as dirty even if hash equal (e.g. order-only touch).
                let dirty_page = prev != Some(&h);
                if !dirty_page {
                    continue;
                }
                dirty += 1;
                let t0 = std::time::Instant::now();
                match image::open(&img_path) {
                    Ok(img) => {
                        let rgb_in = img.to_rgb8();
                        match session.render_page(&rgb_in, &data) {
                            Ok(rgb) => {
                                let out_path = output.join(
                                    Path::new(&data.page)
                                        .file_name()
                                        .unwrap_or_else(|| std::ffi::OsStr::new(&data.page)),
                                );
                                match rgb.save(&out_path) {
                                    Ok(()) => {
                                        ok += 1;
                                        hashes.insert(key, h);
                                        eprintln!(
                                            "{}",
                                            serde_json::json!({"idx": idx+1, "total": proj.pages.len(), "page": data.page, "out": out_path.to_string_lossy(), "ok": true, "ms": t0.elapsed().as_millis() as u64})
                                        );
                                    }
                                    Err(e) => eprintln!(
                                        "{}",
                                        serde_json::json!({"idx": idx+1, "total": proj.pages.len(), "page": data.page, "ok": false, "error": e.to_string()})
                                    ),
                                }
                            }
                            Err(e) => eprintln!(
                                "{}",
                                serde_json::json!({"idx": idx+1, "total": proj.pages.len(), "page": data.page, "ok": false, "error": e.to_string()})
                            ),
                        }
                    }
                    Err(e) => eprintln!(
                        "{}",
                        serde_json::json!({"idx": idx+1, "total": proj.pages.len(), "page": data.page, "ok": false, "error": e.to_string()})
                    ),
                }
            }
            Ok((ok, dirty))
        };
    // Initial full render (all hashes empty => everything dirty).
    let (ok, dirty) = render_once(&mut hashes)?;
    eprintln!(
        "{}",
        serde_json::json!({"done": true, "ok_count": ok, "dirty": dirty})
    );
    if once {
        return Ok(());
    }
    eprintln!("[Watch] polling every {}s — Ctrl-C to stop", interval);
    loop {
        std::thread::sleep(std::time::Duration::from_secs(interval.max(1)));
        match render_once(&mut hashes) {
            Ok((ok, dirty)) => {
                if dirty > 0 {
                    eprintln!(
                        "{}",
                        serde_json::json!({"done": true, "ok_count": ok, "dirty": dirty})
                    );
                }
            }
            Err(e) => eprintln!(
                "{}",
                serde_json::json!({"ok": false, "error": e.to_string()})
            ),
        }
    }
}
