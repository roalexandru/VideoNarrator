//! Shared HTTP client for connection reuse across all API calls.
//! A single `reqwest::Client` reuses TCP connections, TLS sessions, and DNS cache.

static CLIENT: std::sync::LazyLock<reqwest::Client> = std::sync::LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .pool_max_idle_per_host(5)
        .build()
        .expect("Failed to build HTTP client")
});

/// Return a reference to the shared HTTP client.
pub fn shared() -> &'static reqwest::Client {
    &CLIENT
}
