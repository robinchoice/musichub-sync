// What lies in the sync folder now

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::layout::is_noise;
use crate::plan::{DiskFile, IndexEntry};

pub fn sha256(file: &Path) -> io::Result<String> {
    let mut hasher = Sha256::new();
    let mut reader = File::open(file)?;
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn modified_secs(meta: &fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

// Path, size and modification time of a file hashed before, with its hash
pub type Hashed = HashMap<(String, u64, u64), String>;

// Files up to three folders deep (project/track/Versionen/file). Hashes only what the index doesn't
// know unchanged, and only once the file sits still, so a bounce in progress isn't read over and over.
// `hashed` keeps the hashes of files the index never learns (skipped ones) from one round to the next.
pub fn scan(root: &Path, index: &[IndexEntry], now: u64, hashed: &mut Hashed) -> io::Result<Vec<DiskFile>> {
    let mut files = Vec::new();
    walk(root, root, 0, &mut files)?;
    let by_path: HashMap<&str, &IndexEntry> = index.iter().map(|e| (e.path.as_str(), e)).collect();
    let mut still = Hashed::new();
    for file in &mut files {
        let known = by_path.get(file.path.as_str()).is_some_and(|e| e.unchanged(file));
        if !known && now.saturating_sub(file.modified) >= crate::plan::SETTLE_SECS {
            let key = (file.path.clone(), file.size, file.modified);
            file.sha256 = hashed.remove(&key).or_else(|| sha256(&root.join(&file.path)).ok());
            if let Some(sha) = &file.sha256 {
                still.insert(key, sha.clone());
            }
        }
    }
    *hashed = still;
    Ok(files)
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<DiskFile>) -> io::Result<()> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        // A subfolder gone since its parent was listed, e.g. a DAW's temporary folder
        Err(e) if depth > 0 && e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_noise(&name) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            // Gone since the folder was listed, e.g. a DAW's temporary file
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        if meta.is_dir() {
            if depth < 3 {
                walk(root, &entry.path(), depth + 1, out)?;
            }
        } else if meta.is_file() {
            let rel = entry.path().strip_prefix(root).unwrap_or(&entry.path()).components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            out.push(DiskFile { path: rel, size: meta.len(), modified: modified_secs(&meta), sha256: None });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regression_folder_gone_mid_scan_keeps_the_scan_going() {
        let root = tempfile::tempdir().unwrap();
        let mut out = Vec::new();
        assert!(walk(root.path(), &root.path().join("Bounces"), 1, &mut out).is_ok());
        assert!(scan(&root.path().join("missing"), &[], 0, &mut Hashed::new()).is_err());
    }

    #[test]
    fn regression_a_file_the_index_never_learns_is_hashed_only_once() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("Projekt.als"), b"session").unwrap();
        let mut hashed = Hashed::new();
        let first = scan(root.path(), &[], u64::MAX, &mut hashed).unwrap();
        let (key, _) = hashed.iter().next().unwrap();
        let key = key.clone();
        hashed.insert(key, "remembered".into());
        let second = scan(root.path(), &[], u64::MAX, &mut hashed).unwrap();
        assert_ne!(first[0].sha256.as_deref(), Some("remembered"));
        assert_eq!(second[0].sha256.as_deref(), Some("remembered"));
    }
}
