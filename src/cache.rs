use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::translation::CropItem;

/// Prompt signature: hash of genre+custom prompt (like ChunkTranslator.getPromptSignature)
pub fn prompt_signature(custom_prompt: Option<&str>) -> String {
    match custom_prompt {
        Some(p) if !p.trim().is_empty() => {
            let hash = blake3::hash(p.trim().as_bytes());
            format!("custom_{}", &hash.to_hex()[..8])
        }
        _ => "classic".to_string(),
    }
}

pub fn image_hash(image: &image::RgbImage) -> String {
    let raw = image.as_raw();
    let hash = blake3::hash(raw);
    hash.to_hex().to_string()
}

fn cache_db_path() -> PathBuf {
    if let Ok(cache_home) = std::env::var("XDG_CACHE_HOME") {
        PathBuf::from(cache_home).join("kzktdk").join("cache.db")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".cache")
            .join("kzktdk")
            .join("cache.db")
    } else if let Ok(local_appdata) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_appdata).join("kzktdk").join("cache.db")
    } else {
        PathBuf::from("cache.db")
    }
}

pub struct TranslationCache {
    conn: Mutex<Connection>,
}

unsafe impl Send for TranslationCache {}
unsafe impl Sync for TranslationCache {}

impl TranslationCache {
    pub fn open() -> Result<Self> {
        let path = cache_db_path();
        Self::open_at(&path)
    }

    pub fn open_at(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create cache dir {:?}", parent))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open cache DB at {:?}", path))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS translations (
                image_hash TEXT NOT NULL,
                target_lang TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                prompt_sig TEXT NOT NULL,
                translated_text TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (image_hash, target_lang, provider, model, prompt_sig)
            );
            CREATE INDEX IF NOT EXISTS idx_lookup ON translations(image_hash, target_lang);",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS translations (
                image_hash TEXT NOT NULL,
                target_lang TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                prompt_sig TEXT NOT NULL,
                translated_text TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (image_hash, target_lang, provider, model, prompt_sig)
            );",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn get(
        &self,
        image_hash: &str,
        target_lang: &str,
        provider: &str,
        model: &str,
        prompt_sig: &str,
    ) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        let mut stmt = conn
            .prepare(
                "SELECT translated_text FROM translations WHERE image_hash=?1 AND target_lang=?2 AND provider=?3 AND model=?4 AND prompt_sig=?5",
            )
            .ok()?;
        let result: Result<String, _> = stmt.query_row(
            params![image_hash, target_lang, provider, model, prompt_sig],
            |row| row.get(0),
        );
        result.ok()
    }

    pub fn put(
        &self,
        image_hash: &str,
        target_lang: &str,
        provider: &str,
        model: &str,
        prompt_sig: &str,
        translated_text: &str,
    ) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO translations (image_hash, target_lang, provider, model, prompt_sig, translated_text, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![image_hash, target_lang, provider, model, prompt_sig, translated_text, now],
        )?;
        Ok(())
    }

    pub fn clear(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM translations", [])?;
        Ok(())
    }

    /// Filter crops into (cached translations, crops needing translation)
    pub fn filter_cached(
        &self,
        crops: &[CropItem],
        target_lang: &str,
        provider: &str,
        model: &str,
        prompt_sig: &str,
    ) -> (HashMap<String, String>, Vec<CropItem>) {
        let mut cached = HashMap::new();
        let mut to_translate = Vec::new();
        for crop in crops {
            let hash = image_hash(&crop.image);
            if let Some(text) = self.get(&hash, target_lang, provider, model, prompt_sig) {
                cached.insert(crop.id.clone(), text);
            } else {
                to_translate.push(CropItem {
                    id: crop.id.clone(),
                    image: crop.image.clone(),
                });
            }
        }
        (cached, to_translate)
    }

    pub fn save_batch(
        &self,
        translations: &HashMap<String, String>,
        crops: &[CropItem],
        target_lang: &str,
        provider: &str,
        model: &str,
        prompt_sig: &str,
    ) {
        for (id, text) in translations {
            if let Some(crop) = crops.iter().find(|c| &c.id == id) {
                let hash = image_hash(&crop.image);
                let _ = self.put(&hash, target_lang, provider, model, prompt_sig, text);
            }
        }
    }
}
