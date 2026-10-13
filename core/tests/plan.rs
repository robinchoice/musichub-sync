// The folder rules from the plan, one test each

use musichub_sync_core::plan::{plan, Action, DiskFile, IndexEntry, Kind, ProjectRef, SkipReason, TrackRef};
use musichub_sync_core::remote::{Project, Stem, State, Track, Version};

const NOW: u64 = 1_000_000;
const OLD: u64 = NOW - 3600;

fn version(id: &str, number: u32, label: &str, size: u64) -> Version {
    Version {
        id: id.into(),
        version_number: number,
        label: Some(label.into()),
        original_file_name: "bounce.wav".into(),
        file_size: size,
        sha256: Some(format!("hash-{id}")),
    }
}

fn stem(id: &str, name: &str, size: u64, sha: &str) -> Stem {
    Stem { id: id.into(), name: name.into(), original_file_name: format!("{name}.wav"), file_size: size, sha256: Some(sha.into()) }
}

// Tiefgang › Nachtbus with V1 "Rough Mix" and the stem Drums
fn hub() -> State {
    State {
        projects: vec![Project {
            id: "p1".into(),
            name: "Tiefgang".into(),
            can_upload: true,
            tracks: vec![Track {
                id: "t1".into(),
                name: "Nachtbus".into(),
                versions: vec![version("v1", 1, "Rough Mix", 100)],
                stems: vec![stem("s1", "Drums", 50, "drums-1")],
            }],
        }],
    }
}

const V1: &str = "Tiefgang/Nachtbus/Versionen/v1 Rough Mix.wav";
const DRUMS: &str = "Tiefgang/Nachtbus/Stems/Drums.wav";

fn file(path: &str, size: u64, modified: u64, sha: Option<&str>) -> DiskFile {
    DiskFile { path: path.into(), size, modified, sha256: sha.map(Into::into) }
}

fn entry(path: &str, kind: Kind, id: &str, size: u64, sha: &str) -> IndexEntry {
    IndexEntry { path: path.into(), kind, remote_id: id.into(), size, modified: OLD, sha256: Some(sha.into()) }
}

// Both sides as they were after the last sync
fn synced() -> (Vec<IndexEntry>, Vec<DiskFile>) {
    (
        vec![entry(V1, Kind::Version, "v1", 100, "hash-v1"), entry(DRUMS, Kind::Stem, "s1", 50, "drums-1")],
        vec![file(V1, 100, OLD, None), file(DRUMS, 50, OLD, None)],
    )
}

#[test]
fn a_fresh_folder_gets_everything_from_the_hub() {
    let actions = plan(&hub(), &[], &[], NOW);
    assert_eq!(
        actions,
        vec![
            Action::Download { path: DRUMS.into(), kind: Kind::Stem, remote_id: "s1".into() },
            Action::Download { path: V1.into(), kind: Kind::Version, remote_id: "v1".into() },
        ]
    );
}

#[test]
fn nothing_changed_nothing_to_do() {
    let (index, disk) = synced();
    assert_eq!(plan(&hub(), &index, &disk, NOW), vec![]);
}

#[test]
fn files_already_there_are_only_remembered() {
    let disk = vec![file(V1, 100, OLD, Some("hash-v1")), file(DRUMS, 50, OLD, Some("drums-1"))];
    let actions = plan(&hub(), &[], &disk, NOW);
    assert!(actions.contains(&Action::Remember { path: V1.into(), kind: Kind::Version, remote_id: "v1".into() }));
    assert!(actions.contains(&Action::Remember { path: DRUMS.into(), kind: Kind::Stem, remote_id: "s1".into() }));
}

#[test]
fn a_new_bounce_in_versionen_becomes_a_new_version() {
    let (index, mut disk) = synced();
    disk.push(file("Tiefgang/Nachtbus/Versionen/Nachtbus Final.wav", 120, OLD, Some("x")));
    assert_eq!(
        plan(&hub(), &index, &disk, NOW),
        vec![Action::UploadVersion {
            path: "Tiefgang/Nachtbus/Versionen/Nachtbus Final.wav".into(),
            track: TrackRef::Existing("t1".into()),
            label: Some("Nachtbus Final".into()),
            parent_version_id: None,
        }]
    );
}

#[test]
fn a_bounce_still_being_written_waits() {
    let (index, mut disk) = synced();
    disk.push(file("Tiefgang/Nachtbus/Versionen/Nachtbus Final.wav", 120, NOW - 1, None));
    assert_eq!(
        plan(&hub(), &index, &disk, NOW),
        vec![Action::Skip { path: "Tiefgang/Nachtbus/Versionen/Nachtbus Final.wav".into(), reason: SkipReason::Busy }]
    );
}

#[test]
fn saving_over_a_version_makes_a_new_one_on_top() {
    let (index, mut disk) = synced();
    disk[0] = file(V1, 130, OLD + 60, Some("new"));
    assert_eq!(
        plan(&hub(), &index, &disk, NOW),
        vec![Action::UploadVersion {
            path: V1.into(),
            track: TrackRef::Existing("t1".into()),
            label: Some("Rough Mix".into()),
            parent_version_id: Some("v1".into()),
        }]
    );
}

#[test]
fn saving_over_a_stem_gives_it_a_new_file() {
    let (index, mut disk) = synced();
    disk[1] = file(DRUMS, 40, OLD + 60, Some("drums-cut"));
    assert_eq!(plan(&hub(), &index, &disk, NOW), vec![Action::ReplaceStem { path: DRUMS.into(), stem_id: "s1".into() }]);
}

#[test]
fn a_stem_replaced_in_the_hub_comes_down_again() {
    let (index, disk) = synced();
    let mut state = hub();
    state.projects[0].tracks[0].stems[0] = stem("s1", "Drums", 40, "drums-2");
    assert_eq!(plan(&state, &index, &disk, NOW), vec![Action::Download { path: DRUMS.into(), kind: Kind::Stem, remote_id: "s1".into() }]);
}

#[test]
fn a_new_file_in_stems_becomes_a_stem() {
    let (index, mut disk) = synced();
    disk.push(file("Tiefgang/Nachtbus/Stems/Bass.wav", 70, OLD, None));
    assert_eq!(
        plan(&hub(), &index, &disk, NOW),
        vec![Action::UploadStem { path: "Tiefgang/Nachtbus/Stems/Bass.wav".into(), track: TrackRef::Existing("t1".into()), name: "Bass".into() }]
    );
}

#[test]
fn a_mix_dropped_on_the_project_becomes_a_track() {
    let (index, mut disk) = synced();
    disk.push(file("Tiefgang/Kind sein.wav", 70, OLD, None));
    assert_eq!(
        plan(&hub(), &index, &disk, NOW),
        vec![Action::UploadVersion {
            path: "Tiefgang/Kind sein.wav".into(),
            track: TrackRef::New { project: ProjectRef::Existing("p1".into()), name: "Kind sein".into() },
            label: None,
            parent_version_id: None,
        }]
    );
}

#[test]
fn new_folders_become_tracks_and_projects() {
    let (index, mut disk) = synced();
    disk.push(file("Demos 2026/Skizze/Versionen/v1 Idee.wav", 70, OLD, None));
    assert_eq!(
        plan(&hub(), &index, &disk, NOW),
        vec![Action::UploadVersion {
            path: "Demos 2026/Skizze/Versionen/v1 Idee.wav".into(),
            track: TrackRef::New { project: ProjectRef::New("Demos 2026".into()), name: "Skizze".into() },
            label: Some("Idee".into()),
            parent_version_id: None,
        }]
    );
}

#[test]
fn renaming_a_version_file_changes_its_label() {
    let (index, mut disk) = synced();
    disk[0] = file("Tiefgang/Nachtbus/Versionen/v1 Final.wav", 100, OLD + 5, Some("hash-v1"));
    assert_eq!(
        plan(&hub(), &index, &disk, NOW),
        vec![Action::Relabel {
            from: V1.into(),
            to: "Tiefgang/Nachtbus/Versionen/v1 Final.wav".into(),
            version_id: "v1".into(),
            label: Some("Final".into()),
        }]
    );
}

#[test]
fn a_label_changed_in_the_hub_renames_the_local_file() {
    let (index, disk) = synced();
    let mut state = hub();
    state.projects[0].tracks[0].versions[0].label = Some("Final".into());
    assert_eq!(
        plan(&state, &index, &disk, NOW),
        vec![Action::MoveLocal { from: V1.into(), to: "Tiefgang/Nachtbus/Versionen/v1 Final.wav".into() }]
    );
}

#[test]
fn deleting_locally_moves_it_into_the_hubs_trash() {
    let (index, mut disk) = synced();
    disk.remove(1);
    assert_eq!(plan(&hub(), &index, &disk, NOW), vec![Action::TrashRemote { path: DRUMS.into(), kind: Kind::Stem, remote_id: "s1".into() }]);
}

#[test]
fn deleted_in_the_hub_goes_into_the_system_trash() {
    let (index, disk) = synced();
    let mut state = hub();
    state.projects[0].tracks[0].stems.clear();
    assert_eq!(plan(&state, &index, &disk, NOW), vec![Action::TrashLocal { path: DRUMS.into() }]);
}

#[test]
fn deleted_in_the_hub_but_changed_locally_goes_up_again() {
    let (index, mut disk) = synced();
    disk[1] = file(DRUMS, 45, OLD + 60, None);
    let mut state = hub();
    state.projects[0].tracks[0].stems.clear();
    assert_eq!(
        plan(&state, &index, &disk, NOW),
        vec![Action::UploadStem { path: DRUMS.into(), track: TrackRef::Existing("t1".into()), name: "Drums".into() }]
    );
}

#[test]
fn an_unknown_file_where_the_hubs_file_belongs_moves_aside() {
    let disk = vec![file(DRUMS, 99, OLD, Some("other"))];
    let actions = plan(&hub(), &[], &disk, NOW);
    assert!(actions.contains(&Action::KeepAside { path: DRUMS.into(), to: "Tiefgang/Nachtbus/Stems/Drums (lokal).wav".into() }));
    assert!(actions.contains(&Action::Download { path: DRUMS.into(), kind: Kind::Stem, remote_id: "s1".into() }));
}

#[test]
fn other_files_stay_local() {
    let (index, mut disk) = synced();
    disk.push(file("Tiefgang/Nachtbus/Stems/.DS_Store", 1, OLD, None));
    disk.push(file("Tiefgang/Nachtbus/Nachtbus.als", 1, OLD, None));
    disk.push(file("loose.wav", 1, OLD, None));
    assert_eq!(
        plan(&hub(), &index, &disk, NOW),
        vec![
            Action::Skip { path: "Tiefgang/Nachtbus/Nachtbus.als".into(), reason: SkipReason::NotAudio },
            Action::Skip { path: "loose.wav".into(), reason: SkipReason::Unplaced },
        ]
    );
}

#[test]
fn without_upload_rights_new_files_stay_local() {
    let (index, mut disk) = synced();
    let mut state = hub();
    state.projects[0].can_upload = false;
    disk.push(file("Tiefgang/Nachtbus/Stems/Bass.wav", 70, OLD, None));
    assert_eq!(
        plan(&state, &index, &disk, NOW),
        vec![Action::Skip { path: "Tiefgang/Nachtbus/Stems/Bass.wav".into(), reason: SkipReason::NoUploadRights }]
    );
}
