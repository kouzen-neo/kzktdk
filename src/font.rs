use anyhow::{Context, Result, bail};
use ab_glyph::{FontArc, Font};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontInfo {
    pub name: String,
    pub path: String,
    pub has_cjk: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FontRegistryFile {
    pub fonts: HashMap<String, FontInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    pub default_latin: Option<String>,
    pub default_cjk: Option<String>,
}

fn fonts_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/kzktdk/fonts")
    } else if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("kzktdk").join("fonts")
    } else {
        PathBuf::from("fonts")
    }
}

fn registry_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local/share/kzktdk/fonts.json")
    } else {
        PathBuf::from("fonts.json")
    }
}

fn config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/kzktdk/config.toml")
    } else if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join("kzktdk").join("config.toml")
    } else {
        PathBuf::from("config.toml")
    }
}

pub struct FontRegistry;

impl FontRegistry {
    pub fn import(path: &Path, name: Option<String>) -> Result<String> {
        if !path.exists() {
            bail!("Font file not found: {:?}", path);
        }
        let data = std::fs::read(path).with_context(|| format!("Failed to read {:?}", path))?;
        // Validate TTF
        let font = ab_glyph::FontRef::try_from_slice(&data).context("Invalid font file (not TTF/OTF)")?;
        // Check has glyph A
        if font.glyph_id('A').0 == 0 && font.glyph_id('a').0 == 0 {
            bail!("Font has no Latin glyphs");
        }
        let has_cjk = font.glyph_id('あ').0 != 0 || font.glyph_id('中').0 != 0;
        let font_name = name.unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().to_string());
        let dir = fonts_dir();
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(format!("{}.ttf", font_name));
        std::fs::copy(path, &dest).with_context(|| format!("Failed to copy to {:?}", dest))?;

        let mut reg = Self::load_registry();
        reg.fonts.insert(font_name.clone(), FontInfo { name: font_name.clone(), path: dest.to_string_lossy().to_string(), has_cjk });
        Self::save_registry(&reg)?;
        Ok(font_name)
    }

    pub fn list() -> Vec<FontInfo> {
        let reg = Self::load_registry();
        let mut v: Vec<FontInfo> = reg.fonts.values().cloned().collect();
        v.sort_by(|a,b| a.name.cmp(&b.name));
        v
    }

    pub fn remove(name: &str) -> Result<()> {
        let mut reg = Self::load_registry();
        if let Some(info) = reg.fonts.remove(name) {
            let _ = std::fs::remove_file(&info.path);
            Self::save_registry(&reg)?;
            // If default was this font, clear it
            let mut cfg = AppConfig::load();
            if cfg.default_latin.as_deref() == Some(name) {
                cfg.default_latin = None;
                cfg.save()?;
            }
            if cfg.default_cjk.as_deref() == Some(name) {
                cfg.default_cjk = None;
                cfg.save()?;
            }
            Ok(())
        } else {
            bail!("Font '{}' not found in registry", name);
        }
    }

    pub fn resolve(name_or_path: &str) -> Result<FontArc> {
        let p = Path::new(name_or_path);
        if p.exists() {
            let data = std::fs::read(p).with_context(|| format!("Failed to read {:?}", p))?;
            let font = FontArc::try_from_vec(data).context("Invalid font file")?;
            return Ok(font);
        }
        // Try registry by name
        let reg = Self::load_registry();
        if let Some(info) = reg.fonts.get(name_or_path) {
            let data = std::fs::read(&info.path).with_context(|| format!("Failed to read {:?}", info.path))?;
            let font = FontArc::try_from_vec(data).context("Invalid font file in registry")?;
            return Ok(font);
        }
        // Try default embedded
        if name_or_path.eq_ignore_ascii_case("Komika Axis") || name_or_path.eq_ignore_ascii_case("komika") {
            let data = include_bytes!("../fonts/Komika Axis.ttf").to_vec();
            let font = FontArc::try_from_vec(data).unwrap();
            return Ok(font);
        }
        if name_or_path.eq_ignore_ascii_case("KosugiMaru") || name_or_path.eq_ignore_ascii_case("kosugi") {
            let data = include_bytes!("../fonts/KosugiMaru.ttf").to_vec();
            let font = FontArc::try_from_vec(data).unwrap();
            return Ok(font);
        }
        bail!("Font '{}' not found. Run `kzktdk font list` or provide a file path", name_or_path);
    }

    fn load_registry() -> FontRegistryFile {
        let path = registry_path();
        if let Ok(file) = std::fs::File::open(&path) {
            if let Ok(reg) = serde_json::from_reader(file) {
                return reg;
            }
        }
        FontRegistryFile::default()
    }

    fn save_registry(reg: &FontRegistryFile) -> Result<()> {
        let path = registry_path();
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        let tmp = path.with_extension(format!("tmp_{}", std::process::id()));
        let file = std::fs::File::create(&tmp)?;
        serde_json::to_writer_pretty(file, reg)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

impl AppConfig {
    pub fn load() -> Self {
        let path = config_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = toml::from_str(&content) {
                return cfg;
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        let content = toml::to_string_pretty(self).context("Serialize config")?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    pub fn set_latin(name: &str) -> Result<()> {
        let mut cfg = Self::load();
        cfg.default_latin = Some(name.to_string());
        cfg.save()
    }

    pub fn set_cjk(name: &str) -> Result<()> {
        let mut cfg = Self::load();
        cfg.default_cjk = Some(name.to_string());
        cfg.save()
    }

    pub fn get_latin() -> Option<String> { Self::load().default_latin }
    pub fn get_cjk() -> Option<String> { Self::load().default_cjk }
}
