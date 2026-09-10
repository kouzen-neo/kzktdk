use anyhow::Result;
use std::path::PathBuf;

use kzktdk::model::decrypt::decrypt_model;

pub async fn run(source: PathBuf, dest: PathBuf) -> Result<()> {
    decrypt_model(&source, &dest)?;
    println!("Model successfully prepared: {:?}", dest);
    Ok(())
}
