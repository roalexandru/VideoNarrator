//! Lightweight Aptabase telemetry client.
//!
//! Sends anonymous usage events to Aptabase (EU region). No PII is collected.
//! Events are fire-and-forget — failures are silently ignored.

use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const API_URL: &str = "https://eu.aptabase.com/api/v0/events";

fn app_key() -> &'static str {
    static KEY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    KEY.get_or_init(|| {
        std::env::var("NARRATOR_APTABASE_KEY").unwrap_or_else(|_| "A-EU-3488292076".to_string())
    })
}
const SDK_VERSION: &str = concat!("narrator@", env!("CARGO_PKG_VERSION"));

/// Telemetry is fire-and-forget, so nothing ever awaits these requests — which
/// makes an unbounded one a silent leak. `reqwest::Client::new()` sets neither
/// a connect nor a total timeout, so a peer that completes the TCP/TLS
/// handshake and then never answers (captive portals do exactly this) parks the
/// spawned task, its socket and its buffers for the rest of the process. No OS
/// timeout rescues that case, because the connection is established and idle.
/// A desktop app fires telemetry across the whole wizard and roams networks, so
/// these accumulate. Events are also worthless once stale — fail fast instead.
fn build_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .build()
        // Falls back to the untimed default rather than taking the app down;
        // a builder failure here means the TLS backend is unavailable, in
        // which case the requests will fail immediately anyway.
        .unwrap_or_else(|e| {
            tracing::warn!("Telemetry HTTP client build failed ({e}); using default");
            reqwest::Client::new()
        })
}

pub struct TelemetryClient {
    http: reqwest::Client,
    session_id: Mutex<String>,
    app_version: String,
}

impl TelemetryClient {
    pub fn new(app_version: String) -> Arc<Self> {
        Arc::new(Self {
            http: build_http_client(),
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
                .header("App-Key", app_key())
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A peer that completes the TCP handshake and then never answers is the
    /// case no OS timeout covers, so it is the one that used to park a
    /// fire-and-forget telemetry task forever. The outer `timeout` is the
    /// guard: if the client ever loses its own request timeout again, the
    /// inner request hangs, the outer one trips, and this test fails instead
    /// of hanging the suite.
    #[tokio::test]
    async fn telemetry_client_gives_up_on_a_stalled_peer() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Accept, then hold the connection open without ever replying.
        let _server = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });

        let client = build_http_client();
        let started = std::time::Instant::now();

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            client
                .post(format!("http://{addr}/api/v0/events"))
                .json(&serde_json::json!([]))
                .send(),
        )
        .await;

        let inner = outcome.expect("client must time out on its own, not hang the test");
        assert!(
            inner.is_err(),
            "a stalled peer must surface as an error, got a response"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(25),
            "gave up after {:?}, expected the configured ~10s request timeout",
            started.elapsed()
        );
    }
}
