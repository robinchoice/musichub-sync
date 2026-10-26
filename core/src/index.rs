// What both sides held after the last sync, kept in the app's data folder and never in the sync folder:
// a manifest that syncs along can lose entries and silently undo the sync.
// The index belongs to one folder on one hub: read for another folder or hub it would report files as deleted.
// It also belongs to one account: another account's view of the folder would trash what it can't see, so a
// folder synced for someone else is refused, not taken over.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::plan::{IndexEntry, Kind};

#[derive(Serialize, Deserialize)]
struct Stored {
    hub: String,
    // Empty in indexes from before accounts were recorded; the next account to sync takes them over
    #[serde(default)]
    account: String,
    root: String,
    entries: Vec<StoredEntry>,
}

#[derive(Serialize, Deserialize)]
struct StoredEntry {
    path: String,
    kind: String,
    remote_id: String,
    size: u64,
    modified: u64,
    sha256: Option<String>,
}

// One index per hub and folder in `dir`: syncing another folder leaves this one's history and account in place
pub fn file_for(dir: &Path, hub: &str, root: &Path) -> PathBuf {
    let key = format!("{:x}", Sha256::digest(format!("{hub}\n{}", root.to_string_lossy())));
    dir.join(format!("index-{}.json", &key[..16]))
}

// Empty for a hub or folder the index doesn't belong to, an error for a folder synced for another account
pub fn load(file: &Path, hub: &str, account: &str, root: &Path) -> io::Result<Vec<IndexEntry>> {
    let text = match fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let stored: Stored = serde_json::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if stored.hub != hub || stored.root != root.to_string_lossy() {
        return Ok(Vec::new());
    }
    if !stored.account.is_empty() && stored.account != account {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Dieser Ordner wird für {} abgeglichen, wähl für dieses Konto einen anderen", stored.account),
        ));
    }
    Ok(stored
        .entries
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
pub fn save(file: &Path, hub: &str, account: &str, root: &Path, entries: &[IndexEntry]) -> io::Result<()> {
    let entries: Vec<StoredEntry> = entries
        .iter()
        .map(|e| StoredEntry {
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
    let stored = Stored { hub: hub.into(), account: account.into(), root: root.to_string_lossy().into_owned(), entries };
    let tmp = file.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(&stored)?)?;
    fs::rename(tmp, file)
}
