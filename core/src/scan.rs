// What lies in the sync folder now

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

// Files up to three folders deep (project/track/Versionen/file). Hashes only what the index doesn't
// know unchanged, and only once the file sits still, so a bounce in progress isn't read over and over.
pub fn scan(root: &Path, index: &[IndexEntry], now: u64) -> io::Result<Vec<DiskFile>> {
    let mut files = Vec::new();
    walk(root, root, 0, &mut files)?;
    for file in &mut files {
        let known = index.iter().any(|e| e.path == file.path && e.size == file.size && e.modified == file.modified);
        if !known && now.saturating_sub(file.modified) >= crate::plan::SETTLE_SECS {
            file.sha256 = sha256(&root.join(&file.path)).ok();
        }
    }
    Ok(files)
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<DiskFile>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_noise(&name) {
            continue;
        }
        let meta = entry.metadata()?;
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
