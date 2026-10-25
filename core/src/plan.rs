// Compares three views of the sync folder and decides what to do:
//   remote: what the hub holds now (GET /sync/state)
//   index:  what both sides held after the last successful sync
//   disk:   what lies in the folder now
// The plan is pure; the caller runs the actions and updates the index.

use std::collections::{HashMap, HashSet};

use crate::layout::{
    aside_path, is_aside, is_audio, is_noise, parse_version_file, safe_name, stem_file, stem_name, version_file, STEMS, VERSIONS,
};
use crate::remote;

// A file has to sit still this long before it goes up; DAWs write bounces in stages
pub const SETTLE_SECS: u64 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Version,
    Stem,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DiskFile {
    // Relative to the sync folder, separated by '/'
    pub path: String,
    pub size: u64,
    pub modified: u64,
    // Only needed for files the index doesn't know or that changed
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IndexEntry {
    pub path: String,
    pub kind: Kind,
    pub remote_id: String,
    pub size: u64,
    pub modified: u64,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProjectRef {
    Existing(String),
    New(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrackRef {
    Existing(String),
    New { project: ProjectRef, name: String },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SkipReason {
    // Still being written
    Busy,
    NotAudio,
    NoUploadRights,
    // Not inside a project folder, or nested too deep
    Unplaced,
    // The user's own copy "(lokal)" next to the hub's file
    LocalOnly,
    // More than one thing in the hub would take this path; nothing under it is touched on either side
    Collision,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    // Fetch the hub's file to `path`, replacing what's there
    Download { path: String, kind: Kind, remote_id: String },
    // The file on disk already is the hub's file; only the index learns about it
    Remember { path: String, kind: Kind, remote_id: String },
    // The hub renamed it (new label); the local file follows
    MoveLocal { from: String, to: String },
    UploadVersion { path: String, track: TrackRef, label: Option<String>, parent_version_id: Option<String> },
    UploadStem { path: String, track: TrackRef, name: String },
    ReplaceStem { path: String, stem_id: String },
    // The version file was renamed locally: its label changes in the hub
    Relabel { from: String, to: String, version_id: String, label: Option<String> },
    // Deleted locally: into the hub's trash
    TrashRemote { path: String, kind: Kind, remote_id: String },
    // Deleted in the hub: into the system trash
    TrashLocal { path: String },
    // Something that isn't the hub's current file lies where it belongs: it moves aside, then the hub's file comes down
    KeepAside { path: String, to: String, kind: Kind, remote_id: String },
    Skip { path: String, reason: SkipReason },
}

struct Expected<'a> {
    kind: Kind,
    remote_id: &'a str,
    size: u64,
    sha256: Option<&'a str>,
}

fn join(parts: &[&str]) -> String {
    parts.join("/")
}

fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

fn file_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, file)| file)
}

// Paths more than one project, track or file of the hub would take, e.g. two projects named alike.
// Case doesn't count: the file systems of macOS and Windows don't tell "Mix" from "mix" either.
fn collisions(remote: &remote::State) -> Vec<String> {
    fn taken_twice(paths: impl Iterator<Item = String>) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut twice: Vec<String> = Vec::new();
        for path in paths {
            let key = path.to_lowercase();
            if !seen.insert(key.clone()) && !twice.iter().any(|t| t.to_lowercase() == key) {
                twice.push(path);
            }
        }
        twice
    }
    let mut out = taken_twice(remote.projects.iter().map(|p| safe_name(&p.name)));
    for project in &remote.projects {
        let project_dir = safe_name(&project.name);
        out.extend(taken_twice(project.tracks.iter().map(|t| join(&[&project_dir, &safe_name(&t.name)]))));
        for track in &project.tracks {
            let track_dir = join(&[&project_dir, &safe_name(&track.name)]);
            out.extend(taken_twice(track.versions.iter().map(|v| join(&[&track_dir, VERSIONS, &version_file(v)]))));
            out.extend(taken_twice(track.stems.iter().map(|s| join(&[&track_dir, STEMS, &stem_file(s)]))));
        }
    }
    out.sort();
    out.dedup();
    out
}

pub fn plan(remote: &remote::State, index: &[IndexEntry], disk: &[DiskFile], now: u64) -> Vec<Action> {
    // A colliding path stays out on both sides, with everything under it, until someone renames one of the two
    let collisions = collisions(remote);
    let lower: Vec<String> = collisions.iter().map(|c| c.to_lowercase()).collect();
    let collides = |path: &str| {
        let path = path.to_lowercase();
        lower.iter().any(|c| path == *c || path.strip_prefix(c.as_str()).is_some_and(|rest| rest.starts_with('/')))
    };
    let mut actions: Vec<Action> = collisions.iter().map(|c| Action::Skip { path: c.clone(), reason: SkipReason::Collision }).collect();

    // Where every file of the hub belongs
    let mut expected: HashMap<String, Expected> = HashMap::new();
    let mut remote_path: HashMap<&str, String> = HashMap::new();
    let mut colliding_ids: HashSet<&str> = HashSet::new();
    for project in &remote.projects {
        let project_dir = safe_name(&project.name);
        for track in &project.tracks {
            let track_dir = safe_name(&track.name);
            for v in &track.versions {
                let path = join(&[&project_dir, &track_dir, VERSIONS, &version_file(v)]);
                if collides(&path) {
                    colliding_ids.insert(&v.id);
                    continue;
                }
                remote_path.insert(&v.id, path.clone());
                expected.insert(path, Expected { kind: Kind::Version, remote_id: &v.id, size: v.file_size, sha256: v.sha256.as_deref() });
            }
            for s in &track.stems {
                let path = join(&[&project_dir, &track_dir, STEMS, &stem_file(s)]);
                if collides(&path) {
                    colliding_ids.insert(&s.id);
                    continue;
                }
                remote_path.insert(&s.id, path.clone());
                expected.insert(path, Expected { kind: Kind::Stem, remote_id: &s.id, size: s.file_size, sha256: s.sha256.as_deref() });
            }
        }
    }

    // What the hub holds on a colliding path stays out too, wherever its file lies now (e.g. under the project's old name)
    let held: HashSet<&str> = index.iter().filter(|e| colliding_ids.contains(e.remote_id.as_str())).map(|e| e.path.as_str()).collect();
    let index: Vec<IndexEntry> = index.iter().filter(|e| !collides(&e.path) && !held.contains(e.path.as_str())).cloned().collect();
    let disk: Vec<DiskFile> = disk.iter().filter(|d| !collides(&d.path) && !held.contains(d.path.as_str())).cloned().collect();

    let by_path: HashMap<&str, &IndexEntry> = index.iter().map(|e| (e.path.as_str(), e)).collect();
    let by_id: HashMap<&str, &IndexEntry> = index.iter().map(|e| (e.remote_id.as_str(), e)).collect();
    let on_disk: HashMap<&str, &DiskFile> = disk.iter().map(|d| (d.path.as_str(), d)).collect();
    let unchanged = |e: &IndexEntry, d: &DiskFile| e.size == d.size && e.modified == d.modified;
    let settled = |d: &DiskFile| now.saturating_sub(d.modified) >= SETTLE_SECS;

    // Disk paths and index entries already dealt with
    let mut done_disk: HashSet<&str> = HashSet::new();
    let mut done_index: HashSet<&str> = HashSet::new();

    // Version files renamed locally: the old name is gone, a new one with the same number and the very same
    // content appeared. Number and size alone prove nothing: the file may have been saved over as well, and
    // that content has to go up. Until the new file is hashed, both names wait.
    for d in &disk {
        if by_path.contains_key(d.path.as_str()) || expected.contains_key(&d.path) || !file_of(folder_of(&d.path)).eq(VERSIONS) {
            continue;
        }
        let Some((number, label)) = parse_version_file(file_of(&d.path)) else { continue };
        let gone_twin = |e: &&IndexEntry| {
            e.kind == Kind::Version
                && folder_of(&e.path) == folder_of(&d.path)
                && !on_disk.contains_key(e.path.as_str())
                && e.size == d.size
                && parse_version_file(file_of(&e.path)).is_some_and(|(n, _)| n == number)
                && remote_path.contains_key(e.remote_id.as_str())
        };
        if let Some(e) = index.iter().find(|e| gone_twin(e) && e.sha256.is_some() && e.sha256 == d.sha256) {
            actions.push(Action::Relabel {
                from: e.path.clone(),
                to: d.path.clone(),
                version_id: e.remote_id.clone(),
                label: (!label.is_empty()).then_some(label),
            });
            done_disk.insert(&d.path);
            done_index.insert(&e.path);
        } else if d.sha256.is_none() {
            if let Some(e) = index.iter().find(|e| gone_twin(e)) {
                actions.push(Action::Skip { path: d.path.clone(), reason: SkipReason::Busy });
                done_disk.insert(&d.path);
                done_index.insert(&e.path);
            }
        }
    }

    // Every file the hub has
    let mut expected_paths: Vec<&String> = expected.keys().collect();
    expected_paths.sort();
    for path in expected_paths {
        let x = &expected[path];
        let d = on_disk.get(path.as_str()).copied();
        let entry = by_path.get(path.as_str()).copied().filter(|e| e.remote_id == x.remote_id);
        let hub_changed = |e: &IndexEntry| e.size != x.size || matches!((x.sha256, &e.sha256), (Some(a), Some(b)) if a != b);

        // Renamed in the hub: the local file follows, unless it changed or something already sits at the new name
        if let Some(e) = by_id.get(x.remote_id).copied().filter(|e| e.path != *path && !done_index.contains(e.path.as_str())) {
            if d.is_none() && on_disk.get(e.path.as_str()).is_some_and(|old| unchanged(e, old)) {
                done_index.insert(&e.path);
                done_disk.insert(&e.path);
                actions.push(Action::MoveLocal { from: e.path.clone(), to: path.clone() });
                continue;
            }
        }
        if done_index.contains(path.as_str()) && entry.is_some() {
            // Its file was renamed locally, the Relabel above takes care of it
            continue;
        }

        match (entry, d) {
            (Some(e), None) => {
                done_index.insert(&e.path);
                actions.push(Action::TrashRemote { path: path.clone(), kind: x.kind, remote_id: x.remote_id.into() });
            }
            (Some(e), Some(d)) => {
                done_index.insert(&e.path);
                done_disk.insert(&d.path);
                if unchanged(e, d) {
                    if hub_changed(e) {
                        actions.push(Action::Download { path: path.clone(), kind: x.kind, remote_id: x.remote_id.into() });
                    }
                } else if !settled(d) {
                    actions.push(Action::Skip { path: path.clone(), reason: SkipReason::Busy });
                } else if x.kind == Kind::Stem && hub_changed(e) {
                    // Changed on both sides: a stem has no history, so the hub's file stays and the local one moves aside
                    let to = aside_path(path, |p| on_disk.contains_key(p));
                    actions.push(Action::KeepAside { path: path.clone(), to, kind: x.kind, remote_id: x.remote_id.into() });
                } else {
                    // Saved over: a new version on top of this one, or a new file for the stem
                    actions.push(match x.kind {
                        Kind::Version => Action::UploadVersion {
                            path: path.clone(),
                            track: TrackRef::Existing(track_of(remote, x.remote_id).into()),
                            label: parse_version_file(file_of(path)).map(|(_, l)| l).filter(|l| !l.is_empty()),
                            parent_version_id: Some(x.remote_id.into()),
                        },
                        Kind::Stem => Action::ReplaceStem { path: path.clone(), stem_id: x.remote_id.into() },
                    });
                }
            }
            (None, None) => actions.push(Action::Download { path: path.clone(), kind: x.kind, remote_id: x.remote_id.into() }),
            (None, Some(d)) => {
                done_disk.insert(&d.path);
                if let Some(e) = by_path.get(path.as_str()) {
                    done_index.insert(&e.path);
                }
                if d.sha256.is_some() && d.sha256.as_deref() == x.sha256 {
                    actions.push(Action::Remember { path: path.clone(), kind: x.kind, remote_id: x.remote_id.into() });
                } else if !settled(d) {
                    actions.push(Action::Skip { path: path.clone(), reason: SkipReason::Busy });
                } else {
                    let to = aside_path(path, |p| on_disk.contains_key(p));
                    actions.push(Action::KeepAside { path: path.clone(), to, kind: x.kind, remote_id: x.remote_id.into() });
                }
            }
        }
    }

    // Files the hub doesn't have
    for d in &disk {
        if done_disk.contains(d.path.as_str()) {
            continue;
        }
        let name = file_of(&d.path);
        if is_noise(name) {
            continue;
        }
        if is_aside(name) {
            actions.push(Action::Skip { path: d.path.clone(), reason: SkipReason::LocalOnly });
            continue;
        }
        if !is_audio(name) {
            actions.push(Action::Skip { path: d.path.clone(), reason: SkipReason::NotAudio });
            continue;
        }
        if let Some(e) = by_path.get(d.path.as_str()).filter(|e| !done_index.contains(e.path.as_str())) {
            done_index.insert(&e.path);
            if unchanged(e, d) {
                // Gone from the hub
                actions.push(Action::TrashLocal { path: d.path.clone() });
                continue;
            }
        }
        if !settled(d) {
            actions.push(Action::Skip { path: d.path.clone(), reason: SkipReason::Busy });
            continue;
        }
        actions.push(upload_new(remote, d));
    }

    // Index entries whose file is gone on both sides need no action; the caller drops what the plan doesn't mention
    actions
}

fn track_of<'a>(remote: &'a remote::State, version_id: &str) -> &'a str {
    remote
        .projects
        .iter()
        .flat_map(|p| &p.tracks)
        .find(|t| t.versions.iter().any(|v| v.id == version_id))
        .map_or("", |t| t.id.as_str())
}

fn upload_new(remote: &remote::State, d: &DiskFile) -> Action {
    let parts: Vec<&str> = d.path.split('/').collect();
    let project = remote.projects.iter().find(|p| safe_name(&p.name) == parts[0]);
    if parts.len() >= 2 && project.is_some_and(|p| !p.can_upload) {
        return Action::Skip { path: d.path.clone(), reason: SkipReason::NoUploadRights };
    }
    let project_ref = || match project {
        Some(p) => ProjectRef::Existing(p.id.clone()),
        None => ProjectRef::New(parts[0].into()),
    };
    let track_ref = |dir: &str| {
        match project.and_then(|p| p.tracks.iter().find(|t| safe_name(&t.name) == dir)) {
            Some(t) => TrackRef::Existing(t.id.clone()),
            None => TrackRef::New { project: project_ref(), name: dir.into() },
        }
    };
    let file = file_of(&d.path);
    let version_label = || match parse_version_file(file) {
        Some((_, label)) => (!label.is_empty()).then_some(label),
        None => Some(stem_name(file)),
    };

    match parts.as_slice() {
        // A mix dropped on a project becomes a track named after the file, like in the web app
        [_, file] => Action::UploadVersion {
            path: d.path.clone(),
            track: TrackRef::New { project: project_ref(), name: stem_name(file) },
            label: None,
            parent_version_id: None,
        },
        [_, track, _] => Action::UploadVersion { path: d.path.clone(), track: track_ref(track), label: version_label(), parent_version_id: None },
        [_, track, folder, _] if *folder == VERSIONS => {
            Action::UploadVersion { path: d.path.clone(), track: track_ref(track), label: version_label(), parent_version_id: None }
        }
        [_, track, folder, file] if *folder == STEMS => Action::UploadStem { path: d.path.clone(), track: track_ref(track), name: stem_name(file) },
        _ => Action::Skip { path: d.path.clone(), reason: SkipReason::Unplaced },
    }
}
