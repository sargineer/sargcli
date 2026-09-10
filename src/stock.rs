//! `~/.sargineer/stock.yaml`: what is on hand, keyed by sargineer product
//! id (or a slug for consumables). Personal operational state, kept local
//! and git-friendly; identity and knowledge stay on sargineer.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::config::Paths;
use crate::error::{Result, SargError};

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Item {
    pub qty: u32,
    #[serde(rename = "where", skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

pub type Stock = BTreeMap<String, Item>;

pub fn path(paths: &Paths) -> std::path::PathBuf {
    paths.sargineer_dir.join("stock.yaml")
}

pub fn load(paths: &Paths) -> Result<Stock> {
    load_file(&path(paths))
}

pub fn load_file(p: &Path) -> Result<Stock> {
    match fs::read_to_string(p) {
        Ok(text) if text.trim().is_empty() => Ok(Stock::new()),
        Ok(text) => serde_yaml::from_str(&text).map_err(|e| {
            SargError::usage(format!("{} is not a valid stock file: {e}", p.display()))
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Stock::new()),
        Err(e) => Err(e.into()),
    }
}

pub fn save(paths: &Paths, stock: &Stock) -> Result<()> {
    fs::create_dir_all(&paths.sargineer_dir)?;
    let body = serde_yaml::to_string(stock)
        .map_err(|e| SargError::other(anyhow::anyhow!("serialising stock: {e}")))?;
    let header = "# what is on hand, by sargineer product id (or a slug for consumables)\n\
                  # sarg stock set <id> <qty> [--where BIN]\n";
    fs::write(path(paths), format!("{header}{body}"))?;
    Ok(())
}
