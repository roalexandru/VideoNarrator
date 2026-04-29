//! Shared HTTP client for connection reuse across all API calls.
//! A single `reqwest::Client` reuses TCP connections, TLS sessions, and DNS cache.

static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        // 180s tolerates reasoning models (gpt-5, o-series) that routinely
        // take 60–120s to first token. Shorter timeouts caused chunk-level
        // timeouts that retry-stacked into multi-minute generations.
        .timeout(std::time::Duration::from_secs(180))
        .pool_max_idle_per_host(5)
        .build()
        .expect("Failed to build HTTP client")
});

/// Return a reference to the shared HTTP client.
pub fn shared() -> &'static reqwest::Client {
    &CLIENT
}
