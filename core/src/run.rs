// One sync round: fetch the hub's state, scan the folder, plan, carry the plan out, keep the index.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::hub::{Error, Hub, Result};
use crate::layout::{aside_path, file_of, folder_of, safe_name, stem_file, stem_name, version_file, STEMS, VERSIONS};
use crate::plan::{plan, Action, IndexEntry, Kind, ProjectRef, SkipReason, TrackRef};
use crate::{index, remote, scan};

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Default, Clone)]
pub struct Report {
    pub uploaded: Vec<String>,
    pub downloaded: Vec<String>,
    pub problems: Vec<Problem>,
    // Files still being written; they go up in a later round
    pub busy: usize,
    pub skipped: Vec<(String, SkipReason)>,
}

pub struct Syncer<H: Hub> {
    pub hub: H,
    root: PathBuf,
    index_file: PathBuf,
    // Whose folder this is, e.g. the email; the index is bound to it
    account: String,
    // The last state the hub sent; it answers "unchanged" without one
    state: Option<remote::State>,
}

impl<H: Hub> Syncer<H> {
    // The index lives in `index_dir`, one file per hub and folder
    pub fn new(hub: H, root: &Path, index_dir: &Path, account: &str) -> Self {
        let index_file = index::file_for(index_dir, &hub.origin(), root);
        // The single index.json from before is this folder's if hub and folder match; load checks that
        let legacy = index_dir.join("index.json");
        if legacy.exists() && !index_file.exists() {
            let _ = fs::rename(&legacy, &index_file);
        }
        Syncer { hub, root: root.into(), index_file, account: account.into(), state: None }
    }

    pub fn state(&self) -> Option<&remote::State> {
        self.state.as_ref()
    }

    // Projects in `excluded` stay out: neither their files nor the hub's are touched.
    // `stop` is asked before every action; once it says yes the round ends there, like after a lost connection
    pub fn run(&mut self, excluded: &[String], now: u64, stop: impl Fn() -> bool) -> Result<Report> {
        if let Some(state) = self.hub.state()? {
            self.state = Some(state);
        }
        let Some(full) = self.state.clone() else { return Err(Error::Network("Kein Stand vom Hub".into())) };
        // Folders compare without case, like the file systems of macOS and Windows
        let excluded_dirs: Vec<String> = full.projects.iter().filter(|p| excluded.contains(&p.id)).map(|p| safe_name(&p.name).to_lowercase()).collect();
        let outside = |path: &str| excluded_dirs.iter().any(|d| path.split('/').next().is_some_and(|first| first.to_lowercase() == *d));
        // A project sharing its folder with an excluded one stays out as well, else it would write into that folder
        let (shadowed, projects): (Vec<_>, Vec<_>) =
            full.projects.into_iter().filter(|p| !excluded.contains(&p.id)).partition(|p| excluded_dirs.contains(&safe_name(&p.name).to_lowercase()));
        let remote = remote::State { projects };

        fs::create_dir_all(&self.root)?;
        let mut entries: Vec<IndexEntry> = index::load(&self.index_file, &self.hub.origin(), &self.account, &self.root)?;
        let kept: Vec<IndexEntry> = entries.iter().filter(|e| outside(&e.path)).cloned().collect();
        entries.retain(|e| !outside(&e.path));
        let disk: Vec<_> = scan::scan(&self.root, &entries, now)?.into_iter().filter(|d| !outside(&d.path)).collect();

        let mut run = Run { hub: &self.hub, root: &self.root, remote: &remote, entries, report: Report::default(), projects: HashMap::new(), tracks: HashMap::new() };
        run.report.skipped.extend(shadowed.iter().map(|p| (safe_name(&p.name), SkipReason::Collision)));
        // Written after every action: a crash mid-round must not leave an index that calls a renamed file deleted
        let origin = self.hub.origin();
        let save = |entries: &[IndexEntry]| index::save(&self.index_file, &origin, &self.account, &self.root, &[entries, &kept].concat());
        let mut result = Ok(());
        for action in plan(&remote, &run.entries, &disk, now) {
            if stop() {
                break;
            }
            let path = action_path(&action).to_string();
            match run.apply(action) {
                Ok(()) => {}
                Err(e @ (Error::Unauthorized | Error::Network(_))) => {
                    result = Err(e);
                    break;
                }
                Err(e) => run.report.problems.push(Problem { path, message: e.to_string() }),
            }
            save(&run.entries)?;
        }

        // Forget what is gone on both sides
        let ids: HashSet<&str> = remote.projects.iter().flat_map(|p| &p.tracks).flat_map(|t| t.versions.iter().map(|v| v.id.as_str()).chain(t.stems.iter().map(|s| s.id.as_str()))).collect();
        run.entries.retain(|e| ids.contains(e.remote_id.as_str()) || checked_path(&self.root, &e.path).is_ok_and(|p| p.exists()));
        save(&run.entries)?;
        result.map(|()| run.report)
    }
}

fn action_path(action: &Action) -> &str {
    match action {
        Action::Download { path, .. }
        | Action::Remember { path, .. }
        | Action::UploadVersion { path, .. }
        | Action::UploadStem { path, .. }
        | Action::ReplaceStem { path, .. }
        | Action::TrashRemote { path, .. }
        | Action::TrashLocal { path }
        | Action::KeepAside { path, .. }
        | Action::Skip { path, .. } => path,
        Action::MoveLocal { from, .. } | Action::Relabel { from, .. } => from,
    }
}

struct Run<'a, H: Hub> {
    hub: &'a H,
    root: &'a Path,
    remote: &'a remote::State,
    entries: Vec<IndexEntry>,
    report: Report,
    // Created during this round, by name
    projects: HashMap<String, String>,
    tracks: HashMap<(String, String), String>,
}

fn checked_path(root: &Path, path: &str) -> Result<PathBuf> {
    if path.is_empty() || !Path::new(path).components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(Error::Io(io::Error::new(io::ErrorKind::InvalidInput, "Invalid sync path")));
    }
    Ok(root.join(path))
}

impl<H: Hub> Run<'_, H> {
    fn abs(&self, path: &str) -> Result<PathBuf> {
        checked_path(self.root, path)
    }

    fn remember(&mut self, path: &str, kind: Kind, remote_id: &str, sha256: Option<String>) -> Result<()> {
        let meta = fs::metadata(self.abs(path)?)?;
        self.forget(path);
        self.entries.push(IndexEntry {
            path: path.into(),
            kind,
            remote_id: remote_id.into(),
            size: meta.len(),
            modified: scan::modified_secs(&meta),
            sha256,
        });
        Ok(())
    }

    fn forget(&mut self, path: &str) {
        self.entries.retain(|e| e.path != path);
    }

    fn rename(&self, from: &str, to: &str) -> Result<()> {
        if from == to {
            return Ok(());
        }
        let source = self.abs(from)?;
        let target = self.abs(to)?;
        fs::create_dir_all(target.parent().unwrap())?;
        fs::rename(source, target)?;
        Ok(())
    }

    // Puts a local file out of the hub's way, never over another file
    fn move_aside(&self, from: &str, to: &str) -> Result<()> {
        if self.abs(to)?.exists() {
            return Err(Error::Io(io::Error::new(io::ErrorKind::AlreadyExists, format!("{to} ist schon da"))));
        }
        self.rename(from, to)
    }

    fn remote_sha(&self, id: &str) -> Option<String> {
        self.remote.projects.iter().flat_map(|p| &p.tracks).find_map(|t| {
            t.versions.iter().find(|v| v.id == id).map(|v| v.sha256.clone()).or_else(|| t.stems.iter().find(|s| s.id == id).map(|s| s.sha256.clone()))
        })?
    }

    fn track_of_stem(&self, stem_id: &str) -> Option<String> {
        self.remote.projects.iter().flat_map(|p| &p.tracks).find(|t| t.stems.iter().any(|s| s.id == stem_id)).map(|t| t.id.clone())
    }

    fn sha_of(&self, path: &str) -> Result<String> {
        Ok(scan::sha256(&self.abs(path)?)?)
    }

    fn download(&mut self, path: &str, kind: Kind, remote_id: &str) -> Result<()> {
        let dir = folder_of(path);
        let target = self.abs(path)?;
        fs::create_dir_all(self.abs(dir)?)?;
        let name = file_of(path);
        let part = self.abs(&format!("{dir}/.musichub-{name}.part"))?;
        if let Err(e) = self.hub.download(kind, remote_id, &part) {
            let _ = fs::remove_file(&part);
            return Err(e);
        }
        fs::rename(&part, target)?;
        let sha = match self.remote_sha(remote_id) {
            Some(sha) => sha,
            None => self.sha_of(path)?,
        };
        self.remember(path, kind, remote_id, Some(sha))?;
        self.report.downloaded.push(path.into());
        Ok(())
    }

    fn project(&mut self, project: &ProjectRef) -> Result<String> {
        match project {
            ProjectRef::Existing(id) => Ok(id.clone()),
            ProjectRef::New(name) => {
                if let Some(id) = self.projects.get(name) {
                    return Ok(id.clone());
                }
                let id = self.hub.create_project(name)?;
                self.projects.insert(name.clone(), id.clone());
                Ok(id)
            }
        }
    }

    fn track(&mut self, track: &TrackRef) -> Result<String> {
        match track {
            TrackRef::Existing(id) => Ok(id.clone()),
            TrackRef::New { project, name } => {
                let project_id = self.project(project)?;
                let key = (project_id.clone(), name.clone());
                if let Some(id) = self.tracks.get(&key) {
                    return Ok(id.clone());
                }
                let id = self.hub.create_track(&project_id, name)?;
                self.tracks.insert(key, id.clone());
                Ok(id)
            }
        }
    }

    // The track's folder for a file somewhere inside it; a file straight in the project folder became its own track
    fn track_dir(path: &str) -> String {
        let parts: Vec<&str> = path.split('/').collect();
        match parts.as_slice() {
            [project, file] => format!("{project}/{}", safe_name(&stem_name(file))),
            [project, track, ..] => format!("{project}/{track}"),
            _ => String::new(),
        }
    }

    fn apply(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Download { path, kind, remote_id } => self.download(&path, kind, &remote_id),
            Action::Remember { path, kind, remote_id } => {
                let sha = self.sha_of(&path)?;
                self.remember(&path, kind, &remote_id, Some(sha))
            }
            Action::MoveLocal { from, to } => {
                let entry = self.entries.iter().find(|e| e.path == from).cloned();
                self.rename(&from, &to)?;
                self.forget(&from);
                if let Some(e) = entry {
                    self.remember(&to, e.kind, &e.remote_id, e.sha256)?;
                }
                Ok(())
            }
            Action::UploadVersion { path, track, label, parent_version_id } => {
                let track_id = self.track(&track)?;
                let sha = self.sha_of(&path)?;
                let version = self.hub.upload_version(&track_id, &self.abs(&path)?, label.as_deref(), parent_version_id.as_deref(), &sha)?;
                let to = format!("{}/{VERSIONS}/{}", Self::track_dir(&path), version_file(&version));
                // Saved over an existing version: the old one comes down again in the next round
                let target = if !self.abs(&to)?.exists() || to == path { to } else { path.clone() };
                self.rename(&path, &target)?;
                self.forget(&path);
                self.remember(&target, Kind::Version, &version.id, Some(sha))?;
                self.report.uploaded.push(target);
                Ok(())
            }
            Action::UploadStem { path, track, name } => {
                let track_id = self.track(&track)?;
                let sha = self.sha_of(&path)?;
                let stem = self.hub.upload_stem(&track_id, &self.abs(&path)?, &name, &sha)?;
                let to = format!("{}/{STEMS}/{}", Self::track_dir(&path), stem_file(&stem));
                let target = if !self.abs(&to)?.exists() || to == path { to } else { path.clone() };
                self.rename(&path, &target)?;
                self.forget(&path);
                self.remember(&target, Kind::Stem, &stem.id, Some(sha))?;
                self.report.uploaded.push(target);
                Ok(())
            }
            Action::ReplaceStem { path, stem_id } => {
                let track_id = self.track_of_stem(&stem_id).unwrap_or_default();
                let sha = self.sha_of(&path)?;
                match self.hub.replace_stem(&track_id, &stem_id, &self.abs(&path)?, &sha) {
                    Ok(_) => {
                        self.remember(&path, Kind::Stem, &stem_id, Some(sha))?;
                        self.report.uploaded.push(path);
                        Ok(())
                    }
                    // The track is open: its stems stay as everyone agreed to share them; the local file moves aside
                    Err(Error::Conflict(message)) => {
                        let aside = aside_path(&path, |p| self.abs(p).is_ok_and(|p| p.exists()));
                        self.move_aside(&path, &aside)?;
                        self.forget(&path);
                        self.download(&path, Kind::Stem, &stem_id)?;
                        self.report.problems.push(Problem { path: aside, message });
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            Action::Relabel { from, to, version_id, label } => {
                self.hub.relabel(&version_id, label.as_deref())?;
                let sha = self.entries.iter().find(|e| e.path == from).and_then(|e| e.sha256.clone());
                self.forget(&from);
                self.remember(&to, Kind::Version, &version_id, sha)
            }
            Action::TrashRemote { path, kind, remote_id } => match self.hub.trash(kind, &remote_id) {
                Ok(()) => {
                    self.forget(&path);
                    Ok(())
                }
                // Only owners delete versions: the file comes back in the next round
                Err(Error::Forbidden) => {
                    self.forget(&path);
                    self.report.problems.push(Problem { path, message: "Darf nur der Owner löschen, die Datei kommt zurück".into() });
                    Ok(())
                }
                Err(e) => Err(e),
            },
            Action::TrashLocal { path } => {
                trash::delete(self.abs(&path)?).map_err(|e| Error::Io(std::io::Error::other(e.to_string())))?;
                self.forget(&path);
                Ok(())
            }
            Action::KeepAside { path, to, kind, remote_id } => {
                self.move_aside(&path, &to)?;
                // Nothing of the hub's lies here now; should the download fail, the next round fetches it again
                self.forget(&path);
                self.download(&path, kind, &remote_id)?;
                self.report.problems.push(Problem { path: to, message: "Deine Fassung liegt daneben, die Datei aus dem Hub ist aktuell".into() });
                Ok(())
            }
            Action::Skip { path, reason } => {
                if reason == SkipReason::Busy {
                    self.report.busy += 1;
                } else {
                    self.report.skipped.push((path, reason));
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_paths_reject_traversal_and_absolute_paths() {
        let root = Path::new("sync");
        for path in ["", ".", "..", "../outside.wav", "Project/../outside.wav", "/outside.wav"] {
            assert!(matches!(checked_path(root, path), Err(Error::Io(e)) if e.kind() == io::ErrorKind::InvalidInput), "{path:?}");
        }
        #[cfg(windows)]
        for path in [r"C:\outside.wav", r"C:outside.wav", r"\\server\share\outside.wav", r"Project\..\outside.wav"] {
            assert!(checked_path(root, path).is_err(), "{path:?}");
        }
        assert_eq!(checked_path(root, "Project/Track/Stems/Drums.wav").unwrap(), root.join("Project/Track/Stems/Drums.wav"));
    }
}
