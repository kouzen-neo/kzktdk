use anyhow::Result;

use super::args::FontCmd;

pub async fn run(cmd: FontCmd) -> Result<()> {
    match cmd {
        FontCmd::Import { path, name } => {
            let imported = kzktdk::font::FontRegistry::import(&path, name)?;
            println!("Imported font '{}' from {:?}", imported, path);
        }
        FontCmd::List { format, quiet } => {
            let as_json = format == "json";
            let fonts = kzktdk::font::FontRegistry::list();
            if as_json {
                let out = serde_json::json!({
                    "fonts": fonts,
                    "default_latin": kzktdk::font::AppConfig::get_latin(),
                    "default_cjk": kzktdk::font::AppConfig::get_cjk(),
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
                return Ok(());
            }
            if quiet {
                eprintln!("Imported fonts: {}", fonts.len());
                for f in &fonts {
                    eprintln!("  - {} ({} has_cjk={})", f.name, f.path, f.has_cjk);
                }
                return Ok(());
            }
            if fonts.is_empty() {
                println!("No imported fonts. Default: Komika Axis, KosugiMaru (embedded)");
            } else {
                println!("Imported fonts:");
                for f in fonts {
                    println!("  - {} ({} has_cjk={})", f.name, f.path, f.has_cjk);
                }
            }
            println!("Default: Komika Axis, KosugiMaru (embedded)");
            if let Some(def) = kzktdk::font::AppConfig::get_latin() {
                println!("Global default latin: {}", def);
            }
            if let Some(def) = kzktdk::font::AppConfig::get_cjk() {
                println!("Global default cjk: {}", def);
            }
        }
        FontCmd::Remove { name } => {
            kzktdk::font::FontRegistry::remove(&name)?;
            println!("Removed font '{}'", name);
        }
        FontCmd::SetDefault { name, cjk } => {
            if cjk {
                kzktdk::font::AppConfig::set_cjk(&name)?;
                println!("Set global default CJK font to '{}'", name);
            } else {
                kzktdk::font::AppConfig::set_latin(&name)?;
                println!("Set global default Latin font to '{}'", name);
            }
        }
        FontCmd::GetDefault { format, quiet } => {
            let as_json = format == "json";
            let latin = kzktdk::font::AppConfig::get_latin();
            let cjk = kzktdk::font::AppConfig::get_cjk();
            if as_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &serde_json::json!({"default_latin": latin, "default_cjk": cjk})
                    )
                    .unwrap()
                );
                return Ok(());
            }
            if quiet {
                if let Some(def) = latin {
                    eprintln!("Global default latin: {}", def);
                } else {
                    eprintln!("Global default latin: Komika Axis (embedded)");
                }
                if let Some(def) = cjk {
                    eprintln!("Global default cjk: {}", def);
                } else {
                    eprintln!("Global default cjk: KosugiMaru (embedded)");
                }
                return Ok(());
            }
            if let Some(def) = latin {
                println!("Global default latin: {}", def);
            } else {
                println!("Global default latin: Komika Axis (embedded)");
            }
            if let Some(def) = cjk {
                println!("Global default cjk: {}", def);
            } else {
                println!("Global default cjk: KosugiMaru (embedded)");
            }
        }
    }
    Ok(())
}
