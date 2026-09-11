use anyhow::{Context, Result, bail};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;
use zip::{ZipArchive, ZipWriter};

pub enum PreparedInput {
    SingleImage(PathBuf),
    Batch {
        images: Vec<PathBuf>,
        _temp_guard: Option<TempDir>,
        is_archive: bool,
        is_pdf: bool,
        original_name: String,
    },
}

pub fn is_image_path(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        matches!(
            ext.to_lowercase().as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "bmp"
        )
    } else {
        false
    }
}

pub fn is_epub_path(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        ext.eq_ignore_ascii_case("epub")
    } else {
        false
    }
}

pub fn is_pdf_path(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        ext.eq_ignore_ascii_case("pdf")
    } else {
        false
    }
}

pub fn is_archive_path(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        matches!(ext.to_lowercase().as_str(), "cbz" | "zip" | "epub")
    } else {
        false
    }
}

/// Recursively discovers all image files in a directory, sorted naturally (alphanumeric order).
pub fn find_images_in_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut images = Vec::new();
    walk_dir(dir, &mut images)?;
    images.sort_by(|a, b| natord::compare(&a.to_string_lossy(), &b.to_string_lossy()));
    Ok(images)
}

fn walk_dir(dir: &Path, acc: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, acc)?;
        } else if is_image_path(&path) {
            acc.push(path);
        }
    }
    Ok(())
}

/// Extracts a CBZ/ZIP/EPUB archive to a temporary directory and collects images in natural order.
/// For EPUB, skips META-INF and mimetype, extracts embedded images.
pub fn extract_archive(archive_path: &Path) -> Result<(Vec<PathBuf>, TempDir)> {
    let temp_dir =
        TempDir::new().context("Failed to create temporary directory for archive extraction")?;
    let file = File::open(archive_path)
        .with_context(|| format!("Failed to open archive: {}", archive_path.display()))?;
    let mut archive = ZipArchive::new(file)
        .with_context(|| format!("Failed to parse zip archive: {}", archive_path.display()))?;

    let mut extracted_images = Vec::new();

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();

        if file.is_dir() {
            continue;
        }

        // EPUB: skip non-image manifest files
        let lower = name.to_lowercase();
        if lower.starts_with("meta-inf/") || lower == "mimetype" {
            continue;
        }

        let is_image = is_image_path(Path::new(&name));
        if is_image {
            // Flatten path name to avoid nested path errors while preserving uniqueness
            let safe_name = name.replace(['/', '\\'], "_");
            let out_path = temp_dir.path().join(&safe_name);

            let mut out_file = File::create(&out_path)?;
            std::io::copy(&mut file, &mut out_file)?;
            extracted_images.push(out_path);
        }
    }

    extracted_images.sort_by(|a, b| natord::compare(&a.to_string_lossy(), &b.to_string_lossy()));

    if extracted_images.is_empty() {
        bail!(
            "No valid image files found inside archive: {}",
            archive_path.display()
        );
    }

    Ok((extracted_images, temp_dir))
}

/// Universal input resolver: handles single image, folder of images, or CBZ/ZIP archive.
pub fn prepare_input(input_path: &Path) -> Result<PreparedInput> {
    if !input_path.exists() {
        bail!("Input path does not exist: {}", input_path.display());
    }

    if input_path.is_dir() {
        let images = find_images_in_dir(input_path)?;
        if images.is_empty() {
            bail!(
                "No supported images found in folder: {}",
                input_path.display()
            );
        }
        let folder_name = input_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("manga")
            .to_string();

        Ok(PreparedInput::Batch {
            images,
            _temp_guard: None,
            is_archive: false,
            is_pdf: false,
            original_name: folder_name,
        })
    } else if is_archive_path(input_path) {
        let (images, temp_dir) = extract_archive(input_path)?;
        let archive_stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("manga")
            .to_string();

        Ok(PreparedInput::Batch {
            images,
            _temp_guard: Some(temp_dir),
            is_archive: true,
            is_pdf: false,
            original_name: archive_stem,
        })
    } else if is_pdf_path(input_path) {
        let (images, temp_dir) = extract_pdf_pages(input_path)?;
        let stem = input_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("manga")
            .to_string();

        Ok(PreparedInput::Batch {
            images,
            _temp_guard: Some(temp_dir),
            is_archive: true,
            is_pdf: true,
            original_name: stem,
        })
    } else if is_image_path(input_path) {
        Ok(PreparedInput::SingleImage(input_path.to_path_buf()))
    } else {
        bail!(
            "Unsupported file format: {}. Expected image (.jpg, .png, .webp), comic archive (.cbz, .zip, .epub), PDF (.pdf), or a folder.",
            input_path.display()
        );
    }
}

/// Creates a CBZ (ZIP) archive from a list of images.
pub fn create_cbz(image_paths: &[PathBuf], output_path: &Path) -> Result<()> {
    if image_paths.is_empty() {
        bail!("No images to pack into CBZ");
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let file = File::create(output_path)
        .with_context(|| format!("Failed to create output CBZ: {}", output_path.display()))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    for (idx, path) in image_paths.iter().enumerate() {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("jpg");
        let entry_name = format!("page_{:04}.{}", idx + 1, ext);

        zip.start_file(entry_name, options)?;
        let mut f = File::open(path)?;
        let mut buffer = Vec::new();
        f.read_to_end(&mut buffer)?;
        zip.write_all(&buffer)?;
    }

    zip.finish()?;
    Ok(())
}

/// Process-wide Pdfium handle (`Pdfium: Send + Sync`).
/// The native library can only be bound once per process: a second
/// `bind_to_library` fails with `PdfiumLibraryBindingsAlreadyInitialized`.
/// Without this cache, any run touching PDF twice (e.g. PDF input +
/// `--export pdf`, or the round-trip test) would break on the second bind.
static PDFIUM: once_cell::sync::OnceCell<pdfium_render::prelude::Pdfium> =
    once_cell::sync::OnceCell::new();

/// Binds the Pdfium native library (once per process; reused afterwards).
///
/// Resolution order: `PDFIUM_LIB_PATH` env var -> system library.
/// Returns a descriptive error when no library is found (see DOCUMENTATION.md).
fn bind_pdfium() -> Result<&'static pdfium_render::prelude::Pdfium> {
    use pdfium_render::prelude::Pdfium;
    PDFIUM.get_or_try_init(|| {
        if let Ok(custom) = std::env::var("PDFIUM_LIB_PATH") {
            let bindings = Pdfium::bind_to_library(custom.clone()).with_context(|| {
                format!(
                    "Failed to load Pdfium from PDFIUM_LIB_PATH={:?}. Install libpdfium for your platform (see DOCUMENTATION.md).",
                    custom
                )
            })?;
            return Ok(Pdfium::new(bindings));
        }
        let bindings = Pdfium::bind_to_system_library().with_context(|| {
            "Pdfium library not found. Install libpdfium (Linux: libpdfium.so, macOS: libpdfium.dylib, Windows: pdfium.dll) or set PDFIUM_LIB_PATH to its full path. See DOCUMENTATION.md."
                .to_string()
        })?;
        Ok(Pdfium::new(bindings))
    })
}

/// Renders each PDF page to a PNG (~200 DPI target width) in a temp dir.
/// Returns `(page_images, TempDir)` in natural page order (`page_0001.png`).
pub fn extract_pdf_pages(pdf_path: &Path) -> Result<(Vec<PathBuf>, TempDir)> {
    use pdfium_render::prelude::PdfRenderConfig;
    let temp_dir =
        TempDir::new().context("Failed to create temporary directory for PDF extraction")?;
    let pdfium = bind_pdfium()?;
    let doc = pdfium
        .load_pdf_from_file(pdf_path, None)
        .with_context(|| format!("Failed to open PDF: {}", pdf_path.display()))?;
    let pages = doc.pages();
    if pages.is_empty() {
        bail!("PDF has no pages: {}", pdf_path.display());
    }
    let mut images = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        let bitmap = page
            .render_with_config(
                &PdfRenderConfig::new()
                    .set_target_width(1600)
                    .set_maximum_height(2200),
            )
            .with_context(|| format!("Failed to render PDF page {}", i + 1))?;
        let dynimg = bitmap
            .as_image()
            .with_context(|| format!("Failed to decode rendered PDF page {}", i + 1))?;
        let out = temp_dir.path().join(format!("page_{:04}.png", i + 1));
        dynimg
            .save(&out)
            .with_context(|| format!("Failed to save {:?}", out))?;
        images.push(out);
    }
    Ok((images, temp_dir))
}

/// Default translated filename for a PDF input (`doc.pdf` -> `doc_translated.pdf`
/// next to the input when no `-o` is given).
pub fn translated_pdf_name(input: &Path) -> PathBuf {
    let parent = input.parent().unwrap_or(Path::new("."));
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("manga");
    parent.join(format!("{}_translated.pdf", stem))
}

/// Packs rendered page images into a PDF (one full-page image per page,
/// page size = image pixels as points, JPEG quality 90).
pub fn create_pdf(image_paths: &[PathBuf], output_path: &Path) -> Result<()> {
    use pdfium_render::prelude::{PdfPageObjectsCommon, PdfPagePaperSize, PdfPoints};
    if image_paths.is_empty() {
        bail!("No images to pack into PDF");
    }
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let pdfium = bind_pdfium()?;
    let mut doc = pdfium.create_new_pdf()?;
    for path in image_paths {
        let dynimg =
            image::open(path).with_context(|| format!("Failed to open page image {:?}", path))?;
        let (w, h) = (dynimg.width(), dynimg.height());
        let size = PdfPagePaperSize::new_custom(PdfPoints::new(w as f32), PdfPoints::new(h as f32));
        let mut page = doc.pages_mut().create_page_at_end(size)?;
        let obj = pdfium_render::prelude::PdfPageImageObject::new_with_size(
            &doc,
            &dynimg,
            PdfPoints::new(w as f32),
            PdfPoints::new(h as f32),
        )
        .with_context(|| format!("Failed to embed {:?}", path))?;
        page.objects_mut().add_image_object(obj)?;
    }
    doc.save_to_file(output_path)
        .with_context(|| format!("Failed to write PDF {:?}", output_path))?;
    Ok(())
}

#[cfg(test)]
mod pdf_tests {
    use super::*;

    #[test]
    fn pdf_paths_detected() {
        assert!(is_pdf_path(Path::new("a/b.PDF")));
        assert!(!is_pdf_path(Path::new("a/b.cbz")));
        assert_eq!(
            translated_pdf_name(Path::new("ch/ch01.pdf")),
            PathBuf::from("ch/ch01_translated.pdf")
        );
    }

    /// Full PDF round-trip when a Pdfium library is available, soft-skip otherwise.
    #[test]
    fn pdf_roundtrip_or_skip() {
        if bind_pdfium().is_err() {
            eprintln!("SKIP pdf_roundtrip: no Pdfium library (set PDFIUM_LIB_PATH)");
            return;
        }
        let dir = TempDir::new().unwrap();
        // Two synthetic pages.
        let mk = |w: u32, h: u32, v: u8| image::RgbImage::from_pixel(w, h, image::Rgb([v, v, v]));
        let p1 = dir.path().join("a.png");
        let p2 = dir.path().join("b.png");
        mk(100, 140, 250).save(&p1).unwrap();
        mk(120, 140, 20).save(&p2).unwrap();
        let pdf = dir.path().join("in.pdf");
        create_pdf(&[p1, p2], &pdf).unwrap();
        assert!(pdf.is_file());
        let (pages, _guard) = extract_pdf_pages(&pdf).unwrap();
        assert_eq!(pages.len(), 2);
        assert!(
            pages[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("page_0001")
        );
        let out = dir.path().join("out.pdf");
        create_pdf(&pages, &out).unwrap();
        assert!(out.is_file());
    }
}
