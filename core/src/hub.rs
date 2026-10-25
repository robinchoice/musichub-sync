// The hub as the sync needs it. HttpHub talks to the Music Hub API; tests use an in-memory one.

use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::layout::extension;
use crate::plan::Kind;
use crate::remote;

#[derive(Debug)]
pub enum Error {
    // The session is gone: log in again
    Unauthorized,
    Forbidden,
    // E.g. a stem of an open track can't be replaced
    Conflict(String),
    // Storage or upload volume used up
    Full(String),
    Http(u16, String),
    Network(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Unauthorized => write!(f, "Nicht mehr angemeldet"),
            Error::Forbidden => write!(f, "Keine Berechtigung"),
            Error::Conflict(m) | Error::Full(m) => write!(f, "{m}"),
            Error::Http(status, m) => write!(f, "Fehler {status}: {m}"),
            Error::Network(m) => write!(f, "Keine Verbindung: {m}"),
            Error::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub trait Hub {
    // Which hub this is, e.g. its URL; the index is bound to it
    fn origin(&self) -> String;
    // None when nothing changed since the last call
    fn state(&self) -> Result<Option<remote::State>>;
    fn download(&self, kind: Kind, id: &str, to: &Path) -> Result<()>;
    fn create_project(&self, name: &str) -> Result<String>;
    fn create_track(&self, project_id: &str, name: &str) -> Result<String>;
    fn upload_version(&self, track_id: &str, file: &Path, label: Option<&str>, parent_id: Option<&str>, sha256: &str) -> Result<remote::Version>;
    fn upload_stem(&self, track_id: &str, file: &Path, name: &str, sha256: &str) -> Result<remote::Stem>;
    fn replace_stem(&self, track_id: &str, stem_id: &str, file: &Path, sha256: &str) -> Result<remote::Stem>;
    fn relabel(&self, version_id: &str, label: Option<&str>) -> Result<()>;
    fn trash(&self, kind: Kind, id: &str) -> Result<()>;
}

// Same as AUDIO_MIME_BY_EXTENSION in music-hub's shared package
pub fn mime_of(file_name: &str) -> &'static str {
    match extension(file_name).as_deref() {
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("aiff" | "aif" | "aifc") => "audio/aiff",
        _ => "audio/wav",
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub struct HttpHub {
    base: String,
    token: String,
    agent: ureq::Agent,
    etag: std::sync::Mutex<Option<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadUrl {
    upload_url: String,
    file_key: String,
}

#[derive(Deserialize)]
struct Url {
    url: String,
}

#[derive(Deserialize)]
struct Id {
    id: String,
}

impl HttpHub {
    pub fn new(server: &str, token: &str) -> Self {
        HttpHub {
            base: format!("{}/api/v1", server.trim_end_matches('/')),
            token: token.into(),
            // A server that connects and then stays silent, or stops taking an upload, must not hold a round forever;
            // the limits apply per read and write, so uploads and downloads may take as long as they need while bytes flow
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(15))
                .timeout_read(Duration::from_secs(60))
                .timeout_write(Duration::from_secs(60))
                .build(),
            etag: std::sync::Mutex::new(None),
        }
    }

    fn request(&self, method: &str, path: &str) -> ureq::Request {
        self.agent
            .request(method, &format!("{}{path}", self.base))
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("User-Agent", concat!("MusicHubSync/", env!("CARGO_PKG_VERSION")))
    }

    fn send_json(&self, method: &str, path: &str, body: serde_json::Value) -> Result<ureq::Response> {
        checked(self.request(method, path).send_json(body))
    }

    fn get(&self, path: &str) -> Result<ureq::Response> {
        checked(self.request("GET", path).call())
    }

    // Straight to S3 with the presigned URL; size and type are part of the signature
    fn put_file(&self, upload: &UploadUrl, file: &Path) -> Result<()> {
        let size = file.metadata()?.len();
        let response = self
            .agent
            .put(&upload.upload_url)
            .set("Content-Type", mime_of(&file_name(file)))
            .set("Content-Length", &size.to_string())
            .send(File::open(file)?);
        checked(response).map(|_| ())
    }

    // What's new in the user's projects, for the popover; passed on as the API sends it
    pub fn activity(&self, limit: u32) -> Result<serde_json::Value> {
        parse(self.get(&format!("/activity?limit={limit}"))?)
    }

    fn upload_url(&self, path: &str, file: &Path) -> Result<UploadUrl> {
        let name = file_name(file);
        let body = json!({ "fileName": name, "mimeType": mime_of(&name), "fileSize": file.metadata()?.len() });
        parse(self.send_json("POST", path, body)?)
    }
}

fn checked(result: std::result::Result<ureq::Response, ureq::Error>) -> Result<ureq::Response> {
    match result {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(status, response)) => {
            let message = response
                .into_json::<serde_json::Value>()
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
                .unwrap_or_default();
            Err(match status {
                401 => Error::Unauthorized,
                403 => Error::Forbidden,
                409 => Error::Conflict(message),
                413 | 429 | 507 => Error::Full(message),
                _ => Error::Http(status, message),
            })
        }
        Err(e) => Err(Error::Network(e.to_string())),
    }
}

fn parse<T: serde::de::DeserializeOwned>(response: ureq::Response) -> Result<T> {
    response.into_json().map_err(|e| Error::Network(e.to_string()))
}

fn field<T: serde::de::DeserializeOwned>(response: ureq::Response, key: &str) -> Result<T> {
    let mut value: serde_json::Value = parse(response)?;
    serde_json::from_value(value[key].take()).map_err(|e| Error::Network(e.to_string()))
}

impl Hub for HttpHub {
    fn origin(&self) -> String {
        self.base.clone()
    }

    fn state(&self) -> Result<Option<remote::State>> {
        let mut request = self.request("GET", "/sync/state");
        if let Some(etag) = self.etag.lock().unwrap().as_deref() {
            request = request.set("If-None-Match", etag);
        }
        let response = checked(request.call())?;
        if response.status() == 304 {
            return Ok(None);
        }
        let etag = response.header("ETag").map(String::from);
        let state = parse(response)?;
        *self.etag.lock().unwrap() = etag;
        Ok(Some(state))
    }

    fn download(&self, kind: Kind, id: &str, to: &Path) -> Result<()> {
        let path = match kind {
            Kind::Version => format!("/versions/{id}/download-url"),
            Kind::Stem => format!("/stems/{id}/download-url"),
        };
        let url: Url = parse(self.get(&path)?)?;
        let response = checked(self.agent.get(&url.url).call())?;
        let mut out = File::create(to)?;
        io::copy(&mut response.into_reader(), &mut out)?;
        out.sync_all()?;
        Ok(())
    }

    fn create_project(&self, name: &str) -> Result<String> {
        let project: Id = field(self.send_json("POST", "/projects", json!({ "name": name }))?, "project")?;
        Ok(project.id)
    }

    fn create_track(&self, project_id: &str, name: &str) -> Result<String> {
        let track: Id = field(self.send_json("POST", &format!("/tracks/{project_id}"), json!({ "name": name }))?, "track")?;
        Ok(track.id)
    }

    fn upload_version(&self, track_id: &str, file: &Path, label: Option<&str>, parent_id: Option<&str>, sha256: &str) -> Result<remote::Version> {
        let upload = self.upload_url(&format!("/versions/track/{track_id}/upload-url"), file)?;
        self.put_file(&upload, file)?;
        let name = file_name(file);
        let mut body = json!({
            "fileKey": upload.file_key,
            "originalFileName": name,
            "mimeType": mime_of(&name),
            "fileSize": file.metadata()?.len(),
            "sha256": sha256,
        });
        if let Some(label) = label {
            body["label"] = json!(label);
        }
        if let Some(parent) = parent_id {
            body["parentVersionId"] = json!(parent);
        }
        field(self.send_json("POST", &format!("/versions/track/{track_id}"), body)?, "version")
    }

    fn upload_stem(&self, track_id: &str, file: &Path, name: &str, sha256: &str) -> Result<remote::Stem> {
        let upload = self.upload_url(&format!("/stems/track/{track_id}/upload-url"), file)?;
        self.put_file(&upload, file)?;
        let file_name = file_name(file);
        let body = json!({
            "fileKey": upload.file_key,
            "name": name,
            "originalFileName": file_name,
            "mimeType": mime_of(&file_name),
            "fileSize": file.metadata()?.len(),
            "sha256": sha256,
        });
        field(self.send_json("POST", &format!("/stems/track/{track_id}"), body)?, "stem")
    }

    fn replace_stem(&self, track_id: &str, stem_id: &str, file: &Path, sha256: &str) -> Result<remote::Stem> {
        let upload = self.upload_url(&format!("/stems/track/{track_id}/upload-url"), file)?;
        self.put_file(&upload, file)?;
        let name = file_name(file);
        let body = json!({ "fileKey": upload.file_key, "originalFileName": name, "mimeType": mime_of(&name), "sha256": sha256 });
        field(self.send_json("POST", &format!("/stems/{stem_id}/replace"), body)?, "stem")
    }

    fn relabel(&self, version_id: &str, label: Option<&str>) -> Result<()> {
        self.send_json("PATCH", &format!("/versions/{version_id}"), json!({ "label": label })).map(|_| ())
    }

    fn trash(&self, kind: Kind, id: &str) -> Result<()> {
        let path = match kind {
            Kind::Version => format!("/versions/{id}"),
            Kind::Stem => format!("/stems/{id}"),
        };
        checked(self.request("DELETE", &path).call()).map(|_| ())
    }
}
