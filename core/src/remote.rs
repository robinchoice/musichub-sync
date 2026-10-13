use serde::Deserialize;

// What GET /api/v1/sync/state returns: everything the user may mirror, nothing else.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub projects: Vec<Project>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub can_upload: bool,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    pub name: String,
    pub versions: Vec<Version>,
    pub stems: Vec<Stem>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Version {
    pub id: String,
    pub version_number: u32,
    pub label: Option<String>,
    pub original_file_name: String,
    pub file_size: u64,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Stem {
    pub id: String,
    pub name: String,
    pub original_file_name: String,
    pub file_size: u64,
    pub sha256: Option<String>,
}
