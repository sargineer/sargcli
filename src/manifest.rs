//! `sarg.yaml`: the project manifest — the unit of reuse. It names the
//! boards and parts in play, the BOM, and the notes/models the project
//! depends on. Verbs find it by walking up from the working directory the
//! way git finds `.git`, so `sarg ask`, `sarg preflight` and `sarg lesson
//! new` inside a project dir know which hardware they are about.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, SargError};

pub const FILE: &str = "sarg.yaml";

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Manifest {
    /// Display name; defaults to the directory name.
    pub name: String,
    /// The `project` slug sent on lessons (withheld from other readers).
    pub project: String,
    /// The project this one was cloned from (`sarg project new --like`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub like: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    /// Boards by sargineer product id: what `ask`/`preflight` scope to.
    pub boards: Vec<String>,
    /// Other parts in play, by product id.
    pub parts: Vec<String>,
    /// What it takes to build one: product ids or free text, with quantities.
    pub bom: Vec<BomLine>,
    /// Notes and models this project depends on, as <handle>/<id>.
    pub refs: Vec<String>,
    /// Extra terms that must never leave this machine from this project.
    pub deny_terms: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct BomLine {
    /// A sargineer product id, when the line is a known part.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// Free text for anything without a product id (screws, wire, filament).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    pub qty: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Default for BomLine {
    fn default() -> Self {
        BomLine {
            product: None,
            item: None,
            qty: 1,
            note: None,
        }
    }
}

impl BomLine {
    /// The key a stock file or a report uses for this line.
    pub fn key(&self) -> String {
        match (&self.product, &self.item) {
            (Some(p), _) => p.clone(),
            (None, Some(i)) => slugify(i),
            (None, None) => String::new(),
        }
    }
    pub fn label(&self) -> String {
        match (&self.product, &self.item) {
            (Some(p), _) => p.clone(),
            (None, Some(i)) => i.clone(),
            (None, None) => "(empty line)".into(),
        }
    }
}

/// A manifest with the directory it lives in.
#[derive(Clone, Debug)]
pub struct Found {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

impl Found {
    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE)
    }
    pub fn save(&self) -> Result<()> {
        self.manifest.save(&self.dir)
    }
    /// Every product id the project names: boards, parts, BOM products.
    pub fn products(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for id in self
            .manifest
            .boards
            .iter()
            .chain(self.manifest.parts.iter())
            .chain(self.manifest.bom.iter().filter_map(|b| b.product.as_ref()))
        {
            if !out.contains(id) {
                out.push(id.clone());
            }
        }
        out
    }
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<Manifest> {
        let path = dir.join(FILE);
        let text = fs::read_to_string(&path)
            .map_err(|e| SargError::usage(format!("cannot read {}: {e}", path.display())))?;
        serde_yaml::from_str(&text).map_err(|e| {
            SargError::usage(format!("{} is not a valid manifest: {e}", path.display()))
        })
    }

    pub fn save(&self, dir: &Path) -> Result<()> {
        fs::create_dir_all(dir)?;
        let body = serde_yaml::to_string(self)
            .map_err(|e| SargError::other(anyhow::anyhow!("serialising manifest: {e}")))?;
        let header = "# sarg project manifest — boards, parts, BOM and the notes this project\n\
                      # depends on. `sarg project --help`. Product ids are sargineer's.\n";
        fs::write(dir.join(FILE), format!("{header}{body}"))?;
        Ok(())
    }

    /// Add to a list without duplicates; true when it was new.
    pub fn push_unique(list: &mut Vec<String>, id: &str) -> bool {
        let id = id.trim();
        if id.is_empty() || list.iter().any(|x| x == id) {
            return false;
        }
        list.push(id.to_string());
        true
    }

    /// Add or update a BOM line. Returns "added" or "updated".
    pub fn set_bom(
        &mut self,
        product: Option<&str>,
        item: Option<&str>,
        qty: u32,
        note: Option<&str>,
    ) -> &'static str {
        let key = match (product, item) {
            (Some(p), _) => p.to_string(),
            (None, Some(i)) => slugify(i),
            (None, None) => return "ignored",
        };
        if let Some(line) = self.bom.iter_mut().find(|l| l.key() == key) {
            line.qty = qty;
            if note.is_some() {
                line.note = note.map(str::to_string);
            }
            return "updated";
        }
        self.bom.push(BomLine {
            product: product.map(str::to_string),
            item: item.map(str::to_string),
            qty,
            note: note.map(str::to_string),
        });
        "added"
    }
}

/// Walk up from `start` looking for `sarg.yaml`.
pub fn discover_from(start: &Path) -> Option<Found> {
    let mut dir = Some(start.to_path_buf());
    while let Some(d) = dir {
        if d.join(FILE).is_file() {
            if let Ok(manifest) = Manifest::load(&d) {
                return Some(Found { dir: d, manifest });
            }
            return None;
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    None
}

/// The manifest for the working directory, if any. Never an error: verbs
/// that use it fall back to their un-scoped behaviour.
pub fn discover() -> Option<Found> {
    let cwd = std::env::current_dir().ok()?;
    discover_from(&cwd)
}

/// Load from an explicit directory, or discover from cwd.
pub fn find(dir: Option<&Path>) -> Result<Found> {
    match dir {
        Some(d) => {
            let d = if d.as_os_str().is_empty() {
                Path::new(".")
            } else {
                d
            };
            let dir = fs::canonicalize(d)
                .map_err(|e| SargError::usage(format!("{}: {e}", d.display())))?;
            let manifest = Manifest::load(&dir)?;
            Ok(Found { dir, manifest })
        }
        None => discover().ok_or_else(|| {
            SargError::usage(format!(
                "no {FILE} here or above — sarg project init to start one, or give a directory"
            ))
        }),
    }
}

/// `openmv_n6 pantilt` → `openmv-n6-pantilt`
pub fn slugify(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut dash = false;
    for c in s.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slugify("OpenMV N6 speedcam"), "openmv-n6-speedcam");
        assert_eq!(slugify("  xiao_pantilt--v4 "), "xiao-pantilt-v4");
        assert_eq!(slugify("M2.5x6 screws"), "m2-5x6-screws");
    }

    #[test]
    fn bom_lines_key_and_update() {
        let mut m = Manifest::default();
        assert_eq!(m.set_bom(Some("openmv-n6"), None, 1, None), "added");
        assert_eq!(
            m.set_bom(None, Some("M2.5x6 screws"), 8, Some("case lid")),
            "added"
        );
        assert_eq!(m.set_bom(Some("openmv-n6"), None, 2, None), "updated");
        assert_eq!(m.bom.len(), 2);
        assert_eq!(m.bom[0].qty, 2);
        assert_eq!(m.bom[1].key(), "m2-5x6-screws");
        assert_eq!(m.bom[1].label(), "M2.5x6 screws");
    }

    #[test]
    fn roundtrip_yaml() {
        let mut m = Manifest {
            name: "n6".into(),
            project: "openmv-n6".into(),
            ..Default::default()
        };
        m.boards.push("openmv-n6".into());
        m.set_bom(Some("openmv-n6"), None, 1, None);
        let text = serde_yaml::to_string(&m).unwrap();
        assert!(
            !text.contains("like:"),
            "None fields stay out of the file: {text}"
        );
        let back: Manifest = serde_yaml::from_str(&text).unwrap();
        assert_eq!(back.boards, vec!["openmv-n6"]);
        assert_eq!(back.bom[0].qty, 1);
    }
}
