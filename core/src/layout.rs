// Where things live in the sync folder and how file names map back to the hub:
//
//   <Projekt>/<Track>/Versionen/v3 Rough Mix.wav
//   <Projekt>/<Track>/Stems/Drums.wav
//
// The version number belongs to the hub; the rest of a version's file name is its label.

use crate::remote::{Stem, Version};

pub const VERSIONS: &str = "Versionen";
pub const STEMS: &str = "Stems";

// Same list as SUPPORTED_EXTENSIONS in music-hub's shared package
pub const AUDIO_EXTENSIONS: [&str; 6] = ["wav", "mp3", "flac", "aiff", "aif", "aifc"];

pub fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

pub fn file_of(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, file)| file)
}

// Characters no file system takes, and names Windows reserves
pub fn safe_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '-' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if trimmed.is_empty() {
        "Ohne Namen".to_string()
    } else {
        trimmed
    }
}

pub fn extension(file_name: &str) -> Option<String> {
    let (stem, ext) = file_name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    Some(ext.to_lowercase())
}

fn base_name(file_name: &str) -> &str {
    file_name.rsplit_once('.').map_or(file_name, |(stem, _)| stem)
}

pub fn is_audio(file_name: &str) -> bool {
    extension(file_name).is_some_and(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str()))
}

// Files DAWs, editors and operating systems leave behind while they work
pub fn is_noise(file_name: &str) -> bool {
    let lower = file_name.to_lowercase();
    file_name.starts_with('.')
        || file_name.starts_with("~$")
        || lower.ends_with(".tmp")
        || lower.ends_with(".part")
        || lower.ends_with(".crdownload")
        || lower == "desktop.ini"
        || lower == "thumbs.db"
}

// "Bass.wav" → "Bass (lokal).wav", or "Bass (lokal 2).wav" and so on when that name is taken: a copy never replaces another
pub fn aside_path(path: &str, taken: impl Fn(&str) -> bool) -> String {
    let (stem, ext) = match path.rsplit_once('.') {
        Some((stem, ext)) if !stem.ends_with('/') => (stem, format!(".{ext}")),
        _ => (path, String::new()),
    };
    let mut n = 1;
    loop {
        let candidate = if n == 1 { format!("{stem} (lokal){ext}") } else { format!("{stem} (lokal {n}){ext}") };
        if !taken(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

// The user's own copy next to the hub's file; it never goes up
pub fn is_aside(file_name: &str) -> bool {
    let base = base_name(file_name);
    base.ends_with(" (lokal)") || base.rsplit_once(" (lokal ").is_some_and(|(_, rest)| rest.strip_suffix(')').is_some_and(|n| n.parse::<u32>().is_ok()))
}

pub fn label_of(version: &Version) -> String {
    match &version.label {
        Some(label) if !label.trim().is_empty() => label.trim().to_string(),
        _ => base_name(&version.original_file_name).to_string(),
    }
}

fn audio_extension(file_name: &str) -> String {
    extension(file_name).filter(|ext| AUDIO_EXTENSIONS.contains(&ext.as_str())).unwrap_or_else(|| "wav".into())
}

pub fn version_file(version: &Version) -> String {
    let ext = audio_extension(&version.original_file_name);
    format!("v{} {}.{}", version.version_number, safe_name(&label_of(version)), ext)
}

pub fn stem_file(stem: &Stem) -> String {
    let ext = audio_extension(&stem.original_file_name);
    format!("{}.{}", safe_name(&stem.name), ext)
}

// "v3 Final.wav" → (3, "Final"); None for files without a version number in front
pub fn parse_version_file(file_name: &str) -> Option<(u32, String)> {
    let rest = base_name(file_name).strip_prefix(['v', 'V'])?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let number = digits.parse().ok()?;
    let label = rest[digits.len()..].trim_start_matches([' ', '-', '_']).trim().to_string();
    Some((number, label))
}

// The name a stem gets from a file dropped into Stems/
pub fn stem_name(file_name: &str) -> String {
    base_name(file_name).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(number: u32, label: Option<&str>, file: &str) -> Version {
        Version {
            id: "v".into(),
            version_number: number,
            label: label.map(Into::into),
            original_file_name: file.into(),
            file_size: 1,
            sha256: None,
        }
    }

    #[test]
    fn version_files_carry_number_and_label() {
        assert_eq!(version_file(&version(3, Some("Rough Mix"), "bounce.wav")), "v3 Rough Mix.wav");
        assert_eq!(version_file(&version(1, None, "Nachtbus Mix.AIFF")), "v1 Nachtbus Mix.aiff");
        assert_eq!(version_file(&version(2, Some("A/B: laut?"), "x.wav")), "v2 A-B- laut-.wav");
    }

    #[test]
    fn remote_extensions_cannot_add_path_components() {
        for file in ["x./y", "x.\\y", "x.exe", "x.", "x"] {
            assert_eq!(version_file(&version(1, Some("Mix"), file)), "v1 Mix.wav");
            let stem = Stem { id: "s".into(), name: ". .".into(), original_file_name: file.into(), file_size: 1, sha256: None };
            assert_eq!(stem_file(&stem), "Ohne Namen.wav");
        }
    }

    #[test]
    fn a_renamed_version_file_gives_number_and_new_label() {
        assert_eq!(parse_version_file("v3 Final.wav"), Some((3, "Final".into())));
        assert_eq!(parse_version_file("V12 - Master.aif"), Some((12, "Master".into())));
        assert_eq!(parse_version_file("v4.wav"), Some((4, "".into())));
        assert_eq!(parse_version_file("Nachtbus Mix.wav"), None);
        assert_eq!(parse_version_file("vocals.wav"), None);
    }

    #[test]
    fn only_audio_counts_and_noise_stays_out() {
        assert!(is_audio("Drums.WAV"));
        assert!(is_audio("mix.aifc"));
        assert!(!is_audio("session.als"));
        assert!(!is_audio(".wav"));
        assert!(is_noise(".DS_Store"));
        assert!(is_noise("~$mix.wav"));
        assert!(is_noise("bounce.wav.tmp"));
        assert!(!is_noise("Drums.wav"));
    }

    #[test]
    fn copies_put_aside_are_recognised() {
        assert_eq!(aside_path("Tiefgang/Nachtbus/Stems/Bass.wav", |_| false), "Tiefgang/Nachtbus/Stems/Bass (lokal).wav");
        assert_eq!(aside_path("Bass.wav", |p| p == "Bass (lokal).wav"), "Bass (lokal 2).wav");
        assert!(is_aside("Bass (lokal).wav"));
        assert!(is_aside("Bass (lokal 12).aiff"));
        assert!(!is_aside("Bass (lokal x).wav"));
        assert!(!is_aside("Lokal.wav"));
    }

    #[test]
    fn names_stay_valid_on_every_system() {
        assert_eq!(safe_name("Tiefgang: Live?"), "Tiefgang- Live-");
        assert_eq!(safe_name("  ..."), "Ohne Namen");
        assert_eq!(safe_name(".. .."), "Ohne Namen");
        assert_eq!(safe_name(". ."), "Ohne Namen");
        assert_eq!(safe_name(".. \u{2003}.."), "Ohne Namen");
        assert_eq!(safe_name("EP."), "EP");
        assert_eq!(safe_name(".Mix"), ".Mix");
    }
}
