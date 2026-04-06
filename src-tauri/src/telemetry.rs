//! Lightweight Aptabase telemetry client.
//!
//! Sends anonymous usage events to Aptabase (EU region). No PII is collected.
//! Events are fire-and-forget — failures are silently ignored.

use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const APP_KEY: &str = "A-EU-3488292076";
const API_URL: &str = "https://eu.aptabase.com/api/v0/events";
const SDK_VERSION: &str = concat!("narrator@", env!("CARGO_PKG_VERSION"));

pub struct TelemetryClient {
    http: reqwest::Client,
    session_id: Mutex<String>,
    app_version: String,
}

impl TelemetryClient {
    pub fn new(app_version: String) -> Arc<Self> {
        Arc::new(Self {
            http: reqwest::Client::new(),
            session_id: Mutex::new(new_session_id()),
            app_version,
        })
    }

    pub fn track(&self, name: String, props: Option<Value>) {
        let session_id = self
            .session_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let event = serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "sessionId": session_id,
            "eventName": name,
            "systemProps": {
                "isDebug": cfg!(debug_assertions),
                "osName": std::env::consts::OS,
                "osVersion": "",
                "locale": "",
                "appVersion": self.app_version,
                "sdkVersion": SDK_VERSION,
            },
            "props": props,
        });
        // Aptabase API expects an array of events
        let body = serde_json::json!([event]);

        let client = self.http.clone();
        tauri::async_runtime::spawn(async move {
            let _ = client
                .post(API_URL)
                .header("App-Key", APP_KEY)
                .json(&body)
                .send()
                .await;
        });
    }
}

fn new_session_id() -> String {
    let epoch_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Use blake3 hash of timestamp + thread ID as pseudo-random component
    let seed = format!("{}{:?}", epoch_secs, std::thread::current().id());
    let hash = blake3::hash(seed.as_bytes());
    let random =
        u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap_or([0u8; 8])) % 100_000_000;
    (epoch_secs * 100_000_000 + random).to_string()
}
