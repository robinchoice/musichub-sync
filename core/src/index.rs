// What both sides held after the last sync, kept in the app's data folder and never in the sync folder:
// a manifest that syncs along can lose entries and silently undo the sync.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::plan::{IndexEntry, Kind};

#[derive(Serialize, Deserialize)]
struct Stored {
    path: String,
    kind: String,
    remote_id: String,
    size: u64,
    modified: u64,
    sha256: Option<String>,
}

pub fn load(file: &Path) -> io::Result<Vec<IndexEntry>> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let stored: Vec<Stored> = serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(stored
        .into_iter()
        .map(|s| IndexEntry {
            path: s.path,
            kind: if s.kind == "stem" { Kind::Stem } else { Kind::Version },
            remote_id: s.remote_id,
            size: s.size,
            modified: s.modified,
            sha256: s.sha256,
        })
        .collect())
}

// Written next to the target and renamed over it, so a crash never leaves half an index
pub fn save(file: &Path, entries: &[IndexEntry]) -> io::Result<()> {
    let stored: Vec<Stored> = entries
        .iter()
        .map(|e| Stored {
            path: e.path.clone(),
            kind: match e.kind {
                Kind::Version => "version".into(),
                Kind::Stem => "stem".into(),
            },
            remote_id: e.remote_id.clone(),
            size: e.size,
            modified: e.modified,
            sha256: e.sha256.clone(),
        })
        .collect();
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&stored)?)?;
    fs::rename(tmp, file)
}
