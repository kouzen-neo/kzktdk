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

pub fn is_archive_path(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        matches!(ext.to_lowercase().as_str(), "cbz" | "zip")
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

/// Extracts a CBZ or ZIP archive to a temporary directory and collects images in natural order.
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
            original_name: archive_stem,
        })
    } else if is_image_path(input_path) {
        Ok(PreparedInput::SingleImage(input_path.to_path_buf()))
    } else {
        bail!(
            "Unsupported file format: {}. Expected image (.jpg, .png, .webp) or comic archive (.cbz, .zip) or a folder.",
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
