// Login like the DAW plugin: the app shows a code, the user confirms it in the browser
// where they are logged in already, the app polls until it gets its session.

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::hub::{Error, Result};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub poll_interval_seconds: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct User {
    pub email: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub token: String,
    pub user: User,
}

fn api(server: &str, path: &str) -> String {
    format!("{}/api/v1{path}", server.trim_end_matches('/'))
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build()
}

fn error(e: ureq::Error) -> Error {
    match e {
        ureq::Error::Status(status, response) => {
            let message = response
                .into_json::<serde_json::Value>()
                .ok()
                .and_then(|v| v.get("error").and_then(|m| m.as_str()).map(String::from))
                .unwrap_or_default();
            Error::Http(status, message)
        }
        e => Error::Network(e.to_string()),
    }
}

pub fn start(server: &str) -> Result<DeviceCode> {
    agent()
        .post(&api(server, "/auth/device"))
        .send_json(json!({ "client": "Music Hub Sync" }))
        .map_err(error)?
        .into_json()
        .map_err(|e| Error::Network(e.to_string()))
}

// Ok(None) while the user hasn't confirmed yet
pub fn poll(server: &str, code: &DeviceCode) -> Result<Option<Session>> {
    let response = agent()
        .post(&api(server, "/auth/device/token"))
        .send_json(json!({ "deviceCode": code.device_code }))
        .map_err(error)?;
    if response.status() == 202 {
        return Ok(None);
    }
    response.into_json().map(Some).map_err(|e| Error::Network(e.to_string()))
}
