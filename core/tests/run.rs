// Whole sync rounds against an in-memory hub and a temporary folder

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Once;
use std::time::{SystemTime, UNIX_EPOCH};

use musichub_sync_core::hub::{Error, Hub, Result};
use musichub_sync_core::plan::{Kind, SkipReason};
use musichub_sync_core::remote::{Project, State, Stem, Track, Version};
use musichub_sync_core::run::Syncer;
use musichub_sync_core::scan::sha256;

#[derive(Default)]
struct FakeHub {
    state: RefCell<State>,
    files: RefCell<HashMap<String, Vec<u8>>>,
    next: RefCell<u32>,
    // Stems of open tracks can't get a new file
    open_tracks: Vec<String>,
    // Versions only the owner may delete
    owner: bool,
    origin: String,
    // The process dies in the n-th upload, like a crash mid-round
    crash_at_upload: Option<u32>,
    uploads: RefCell<u32>,
}

fn sha(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

impl FakeHub {
    fn new() -> Self {
        let hub = FakeHub { owner: true, ..Default::default() };
        hub.state.borrow_mut().projects.push(Project { id: "p1".into(), name: "Tiefgang".into(), can_upload: true, tracks: vec![] });
        let t = hub.add_track("p1", "Nachtbus");
        hub.add_version(&t, b"mix one", Some("Rough Mix"), None);
        hub.add_stem(&t, "Drums", b"drums");
        hub
    }

    fn id(&self, prefix: &str) -> String {
        *self.next.borrow_mut() += 1;
        format!("{prefix}{}", self.next.borrow())
    }

    fn track_mut<T>(&self, track_id: &str, f: impl FnOnce(&mut Track) -> T) -> T {
        let mut state = self.state.borrow_mut();
        let track = state.projects.iter_mut().flat_map(|p| &mut p.tracks).find(|t| t.id == track_id).expect("track");
        f(track)
    }

    fn add_track(&self, project_id: &str, name: &str) -> String {
        let id = self.id("t");
        let mut state = self.state.borrow_mut();
        let project = state.projects.iter_mut().find(|p| p.id == project_id).expect("project");
        project.tracks.push(Track { id: id.clone(), name: name.into(), versions: vec![], stems: vec![] });
        id
    }

    fn add_version(&self, track_id: &str, bytes: &[u8], label: Option<&str>, file_name: Option<&str>) -> Version {
        let id = self.id("v");
        self.files.borrow_mut().insert(id.clone(), bytes.to_vec());
        self.track_mut(track_id, |t| {
            let version = Version {
                id: id.clone(),
                version_number: t.versions.iter().map(|v| v.version_number).max().unwrap_or(0) + 1,
                label: label.map(Into::into),
                original_file_name: file_name.unwrap_or("bounce.wav").into(),
                file_size: bytes.len() as u64,
                sha256: Some(sha(bytes)),
            };
            t.versions.push(version.clone());
            version
        })
    }

    fn add_stem(&self, track_id: &str, name: &str, bytes: &[u8]) -> Stem {
        let id = self.id("s");
        self.files.borrow_mut().insert(id.clone(), bytes.to_vec());
        let stem = Stem { id, name: name.into(), original_file_name: format!("{name}.wav"), file_size: bytes.len() as u64, sha256: Some(sha(bytes)) };
        self.track_mut(track_id, |t| t.stems.push(stem.clone()));
        stem
    }

    fn versions(&self) -> Vec<Version> {
        self.state.borrow().projects.iter().flat_map(|p| &p.tracks).flat_map(|t| t.versions.clone()).collect()
    }
}

impl Hub for FakeHub {
    fn origin(&self) -> String {
        self.origin.clone()
    }

    fn state(&self) -> Result<Option<State>> {
        Ok(Some(self.state.borrow().clone()))
    }

    fn download(&self, _kind: Kind, id: &str, to: &Path) -> Result<()> {
        fs::write(to, &self.files.borrow()[id])?;
        Ok(())
    }

    fn create_project(&self, name: &str) -> Result<String> {
        let id = self.id("p");
        self.state.borrow_mut().projects.push(Project { id: id.clone(), name: name.into(), can_upload: true, tracks: vec![] });
        Ok(id)
    }

    fn create_track(&self, project_id: &str, name: &str) -> Result<String> {
        Ok(self.add_track(project_id, name))
    }

    fn upload_version(&self, track_id: &str, file: &Path, label: Option<&str>, _parent: Option<&str>, _sha: &str) -> Result<Version> {
        *self.uploads.borrow_mut() += 1;
        if Some(*self.uploads.borrow()) == self.crash_at_upload {
            panic!("crash");
        }
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        Ok(self.add_version(track_id, &fs::read(file)?, label, Some(&name)))
    }

    fn upload_stem(&self, track_id: &str, file: &Path, name: &str, _sha: &str) -> Result<Stem> {
        Ok(self.add_stem(track_id, name, &fs::read(file)?))
    }

    fn replace_stem(&self, track_id: &str, stem_id: &str, file: &Path, _sha: &str) -> Result<Stem> {
        if self.open_tracks.iter().any(|t| t == track_id) {
            return Err(Error::Conflict("Der Track ist offen".into()));
        }
        let bytes = fs::read(file)?;
        self.files.borrow_mut().insert(stem_id.into(), bytes.clone());
        Ok(self.track_mut(track_id, |t| {
            let stem = t.stems.iter_mut().find(|s| s.id == stem_id).unwrap();
            stem.file_size = bytes.len() as u64;
            stem.sha256 = Some(sha(&bytes));
            stem.clone()
        }))
    }

    fn relabel(&self, version_id: &str, label: Option<&str>) -> Result<()> {
        let mut state = self.state.borrow_mut();
        let version = state.projects.iter_mut().flat_map(|p| &mut p.tracks).flat_map(|t| &mut t.versions).find(|v| v.id == version_id).unwrap();
        version.label = label.map(Into::into);
        Ok(())
    }

    fn trash(&self, kind: Kind, id: &str) -> Result<()> {
        if kind == Kind::Version && !self.owner {
            return Err(Error::Forbidden);
        }
        for track in self.state.borrow_mut().projects.iter_mut().flat_map(|p| &mut p.tracks) {
            track.versions.retain(|v| v.id != id);
            track.stems.retain(|s| s.id != id);
        }
        Ok(())
    }
}

const V1: &str = "Tiefgang/Nachtbus/Versionen/v1 Rough Mix.wav";
const DRUMS: &str = "Tiefgang/Nachtbus/Stems/Drums.wav";

// Files written now count as settled
fn later() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 60
}

// The system trash of this test binary lives in a temporary folder, not in the user's
fn private_trash() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let dir = tempfile::tempdir().unwrap().keep();
        std::env::set_var("XDG_DATA_HOME", dir);
    });
}

struct Setup {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    syncer: Syncer<FakeHub>,
}

fn setup(hub: FakeHub) -> Setup {
    private_trash();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Music Hub");
    let syncer = Syncer::new(hub, &root, &dir.path().join("index.json"));
    Setup { _dir: dir, root, syncer }
}

impl Setup {
    fn round(&mut self) -> musichub_sync_core::run::Report {
        self.syncer.run(&[], later()).expect("round")
    }
    fn read(&self, path: &str) -> Vec<u8> {
        fs::read(self.root.join(path)).unwrap()
    }
    fn write(&self, path: &str, bytes: &[u8]) {
        fs::create_dir_all(self.root.join(path).parent().unwrap()).unwrap();
        fs::write(self.root.join(path), bytes).unwrap();
    }
    fn exists(&self, path: &str) -> bool {
        self.root.join(path).exists()
    }
    // A clean round afterwards: nothing left to do
    fn settled(&mut self) {
        let report = self.round();
        assert!(report.uploaded.is_empty() && report.downloaded.is_empty() && report.problems.is_empty(), "{report:?}");
    }
}

#[test]
fn the_first_round_brings_everything_down_the_second_has_nothing_to_do() {
    let mut s = setup(FakeHub::new());
    let report = s.round();
    assert_eq!(report.downloaded.len(), 2);
    assert_eq!(s.read(V1), b"mix one");
    assert_eq!(s.read(DRUMS), b"drums");
    s.settled();
}

#[test]
fn a_bounce_goes_up_and_takes_the_versions_name() {
    let mut s = setup(FakeHub::new());
    s.round();
    s.write("Tiefgang/Nachtbus/Versionen/Nachtbus Final.wav", b"mix two");
    let report = s.round();
    assert_eq!(report.uploaded, vec!["Tiefgang/Nachtbus/Versionen/v2 Nachtbus Final.wav"]);
    assert!(!s.exists("Tiefgang/Nachtbus/Versionen/Nachtbus Final.wav"));
    assert_eq!(s.syncer.hub.versions().last().unwrap().label.as_deref(), Some("Nachtbus Final"));
    s.settled();
}

#[test]
fn saving_over_a_version_adds_one_and_the_old_one_comes_back() {
    let mut s = setup(FakeHub::new());
    s.round();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    s.write(V1, b"mix one, louder");
    s.round();
    assert_eq!(s.read("Tiefgang/Nachtbus/Versionen/v2 Rough Mix.wav"), b"mix one, louder");
    s.round();
    assert_eq!(s.read(V1), b"mix one");
    s.settled();
}

#[test]
fn a_stem_changed_here_and_in_the_hub_comes_down_and_the_local_one_lies_next_to_it() {
    let mut s = setup(FakeHub::new());
    s.round();
    s.syncer.hub.replace_stem("t1", "s3", &{ let f = s._dir.path().join("hub.wav"); fs::write(&f, b"drums, hub cut").unwrap(); f }, "x").unwrap();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    s.write(DRUMS, b"drums, my cut");
    let report = s.round();
    assert_eq!(s.read(DRUMS), b"drums, hub cut");
    assert_eq!(s.read("Tiefgang/Nachtbus/Stems/Drums (lokal).wav"), b"drums, my cut");
    assert_eq!(report.problems[0].path, "Tiefgang/Nachtbus/Stems/Drums (lokal).wav");
    assert!(report.skipped.is_empty());
    let report = s.round();
    assert_eq!(report.skipped, vec![("Tiefgang/Nachtbus/Stems/Drums (lokal).wav".to_string(), SkipReason::LocalOnly)]);
    assert!(report.uploaded.is_empty() && report.problems.is_empty(), "{report:?}");
}

#[test]
fn a_crash_mid_round_does_not_trash_the_version_saved_over_before_it() {
    let mut s = setup(FakeHub { crash_at_upload: Some(2), ..FakeHub::new() });
    s.round();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    s.write(V1, b"mix one, louder");
    s.write("Tiefgang/Nachtbus/Versionen/Nachtbus Final.wav", b"mix two");
    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| s.syncer.run(&[], later())));
    assert!(crashed.is_err());
    // The app starts again
    let hub = std::mem::replace(&mut s.syncer, Syncer::new(FakeHub::default(), &s.root, &s._dir.path().join("index.json"))).hub;
    s.syncer.hub = FakeHub { crash_at_upload: None, ..hub };
    s.round();
    // The first version (id v2) had its file renamed to "v2 Rough Mix.wav" before the crash; that must not count as deleted
    let ids: Vec<String> = s.syncer.hub.versions().iter().map(|v| v.id.clone()).collect();
    assert_eq!(ids, vec!["v2", "v4", "v5"], "{ids:?}");
    assert_eq!(s.read("Tiefgang/Nachtbus/Versionen/v2 Rough Mix.wav"), b"mix one, louder");
}

#[test]
fn renaming_a_version_changes_its_label() {
    let mut s = setup(FakeHub::new());
    s.round();
    fs::rename(s.root.join(V1), s.root.join("Tiefgang/Nachtbus/Versionen/v1 Final.wav")).unwrap();
    s.round();
    assert_eq!(s.syncer.hub.versions()[0].label.as_deref(), Some("Final"));
    s.settled();
    assert!(s.exists("Tiefgang/Nachtbus/Versionen/v1 Final.wav"));
}

#[test]
fn renaming_and_saving_over_a_version_brings_the_new_content_up() {
    let mut s = setup(FakeHub::new());
    s.round();
    let renamed = "Tiefgang/Nachtbus/Versionen/v1 Final.wav";
    fs::rename(s.root.join(V1), s.root.join(renamed)).unwrap();
    s.write(renamed, b"mix two");
    let report = s.round();
    assert_eq!(report.uploaded.len(), 1, "{report:?}");
    let versions = s.syncer.hub.versions();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].label.as_deref(), Some("Final"));
    assert_eq!(s.syncer.hub.files.borrow()[&versions[0].id], b"mix two");
    s.settled();
}

#[test]
fn deleting_locally_empties_it_from_the_hub_unless_only_the_owner_may() {
    let mut s = setup(FakeHub::new());
    s.round();
    fs::remove_file(s.root.join(DRUMS)).unwrap();
    s.round();
    assert!(s.syncer.hub.state.borrow().projects[0].tracks[0].stems.is_empty());

    let mut s = setup(FakeHub { owner: false, ..FakeHub::new() });
    s.round();
    fs::remove_file(s.root.join(V1)).unwrap();
    let report = s.round();
    assert_eq!(report.problems.len(), 1);
    s.round();
    assert_eq!(s.read(V1), b"mix one");
}

#[test]
fn deleted_in_the_hub_leaves_the_folder() {
    let mut s = setup(FakeHub::new());
    s.round();
    s.syncer.hub.trash(Kind::Stem, "s3").unwrap();
    s.round();
    assert!(!s.exists(DRUMS));
    s.settled();
}

#[test]
fn a_stem_of_an_open_track_stays_and_the_local_file_moves_aside() {
    let hub = FakeHub { open_tracks: vec!["t1".into()], ..FakeHub::new() };
    let mut s = setup(hub);
    s.round();
    std::thread::sleep(std::time::Duration::from_millis(1100));
    s.write(DRUMS, b"drums, cut");
    let report = s.round();
    assert_eq!(report.problems[0].path, "Tiefgang/Nachtbus/Stems/Drums (lokal).wav");
    assert_eq!(s.read(DRUMS), b"drums");
    assert_eq!(s.read("Tiefgang/Nachtbus/Stems/Drums (lokal).wav"), b"drums, cut");
}

#[test]
fn a_second_conflict_keeps_the_first_safety_copy() {
    let hub = FakeHub { open_tracks: vec!["t1".into()], ..FakeHub::new() };
    let mut s = setup(hub);
    s.round();
    let aside = "Tiefgang/Nachtbus/Stems/Drums (lokal).wav";
    s.write(aside, b"first cut");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    s.write(DRUMS, b"second cut");
    let report = s.round();
    assert_eq!(report.problems[0].path, "Tiefgang/Nachtbus/Stems/Drums (lokal 2).wav");
    assert_eq!(s.read(aside), b"first cut");
    assert_eq!(s.read("Tiefgang/Nachtbus/Stems/Drums (lokal 2).wav"), b"second cut");
    assert_eq!(s.read(DRUMS), b"drums");
}

#[test]
fn an_unknown_file_moves_aside_before_the_hubs_file_comes_down() {
    let mut s = setup(FakeHub::new());
    s.write(DRUMS, b"my own drums");
    s.write("Tiefgang/Nachtbus/Stems/Drums (lokal).wav", b"older copy");
    s.round();
    assert_eq!(s.read(DRUMS), b"drums");
    assert_eq!(s.read("Tiefgang/Nachtbus/Stems/Drums (lokal).wav"), b"older copy");
    assert_eq!(s.read("Tiefgang/Nachtbus/Stems/Drums (lokal 2).wav"), b"my own drums");
}

#[test]
fn new_folders_become_projects_and_tracks_and_a_loose_mix_a_track() {
    let mut s = setup(FakeHub::new());
    s.round();
    s.write("Demos 2026/Skizze/Stems/Bass.wav", b"bass");
    s.write("Demos 2026/Skizze/Versionen/v1 Idee.wav", b"idea");
    s.write("Tiefgang/Kind sein.wav", b"kind");
    s.round();
    let state = s.syncer.hub.state.borrow().clone();
    let demos = state.projects.iter().find(|p| p.name == "Demos 2026").unwrap();
    assert_eq!(demos.tracks.len(), 1);
    assert_eq!(demos.tracks[0].stems[0].name, "Bass");
    assert!(state.projects[0].tracks.iter().any(|t| t.name == "Kind sein"));
    assert!(s.exists("Tiefgang/Kind sein/Versionen/v1 Kind sein.wav"));
    assert_eq!(sha256(&s.root.join("Demos 2026/Skizze/Stems/Bass.wav")).unwrap(), sha(b"bass"));
    s.settled();
}

#[test]
fn a_new_folder_starts_without_the_old_folders_history() {
    let mut s = setup(FakeHub::new());
    s.round();
    let new_root = s._dir.path().join("New Music Hub");
    let hub = std::mem::replace(&mut s.syncer, Syncer::new(FakeHub::default(), &new_root, &s._dir.path().join("index.json"))).hub;
    s.syncer.hub = hub;
    let report = s.round();
    assert_eq!(report.downloaded.len(), 2);
    assert_eq!(s.syncer.hub.versions().len(), 1);
    assert!(!s.syncer.hub.state.borrow().projects[0].tracks[0].stems.is_empty());
    assert!(s.exists(V1) && new_root.join(V1).exists());
}

#[test]
fn another_hub_starts_without_this_hubs_history() {
    let mut s = setup(FakeHub::new());
    s.round();
    let index = s._dir.path().join("index.json");
    s.syncer = Syncer::new(FakeHub { origin: "other".into(), ..Default::default() }, &s.root, &index);
    s.round();
    assert_eq!(s.read(V1), b"mix one");
    assert_eq!(s.syncer.hub.state.borrow().projects.len(), 1, "the folder's content went up to the other hub");
}

#[test]
fn a_project_named_like_an_excluded_one_stays_out_of_its_folder() {
    let hub = FakeHub::new();
    let twin = hub.create_project("Tiefgang").unwrap();
    let t = hub.add_track(&twin, "Nachtbus");
    hub.add_version(&t, b"twin mix", Some("Rough Mix"), None);
    let mut s = setup(hub);
    s.write(V1, b"my own file");
    let report = s.syncer.run(&[twin], later()).unwrap();
    assert_eq!(s.read(V1), b"my own file");
    assert!(report.downloaded.is_empty());
    assert!(report.skipped.contains(&("Tiefgang".into(), SkipReason::Collision)), "{report:?}");
}

#[test]
fn an_excluded_project_is_left_alone() {
    let mut s = setup(FakeHub::new());
    s.syncer.run(&["p1".into()], later()).unwrap();
    assert!(!s.exists(V1));
    s.write("Tiefgang/Notiz.wav", b"x");
    s.syncer.run(&["p1".into()], later()).unwrap();
    assert_eq!(s.syncer.hub.state.borrow().projects.len(), 1);
    assert!(s.exists("Tiefgang/Notiz.wav"));
}
