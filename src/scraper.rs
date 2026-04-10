//! Scraper trait for extensibility.
//!
//! Implement this trait to add support for new cinema websites.

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use crate::models::TheaterData;

/// Creates a reqwest ClientBuilder with optional SOCKS5 proxy.
///
/// If the `SOCKS_PROXY` env var is set (e.g. `socks5://127.0.0.1:1080`),
/// the client will route requests through it. Otherwise, connects directly.
pub fn http_client_builder() -> reqwest::ClientBuilder {
    let builder = Client::builder();
    if let Ok(proxy_url) = std::env::var("SOCKS_PROXY") {
        let proxy_url = proxy_url.trim();
        if proxy_url.is_empty() {
            return builder;
        }
        match reqwest::Proxy::all(proxy_url) {
            Ok(proxy) => builder.proxy(proxy),
            Err(e) => {
                eprintln!("Warning: invalid SOCKS_PROXY '{}': {}", proxy_url, e);
                builder
            }
        }
    } else {
        builder
    }
}

/// Trait for cinema website scrapers.
///
/// Each implementation handles a different website (UCI, Yorck, etc.).
/// The scraper should fetch all available showtime data for its theaters.
#[async_trait]
pub trait Scraper: Send + Sync {
    /// Unique name for this source (e.g., "uci_kinowelt").
    fn name(&self) -> &str;

    /// Base URL for this source.
    fn url(&self) -> &str;

    /// Scrape all theaters and their showtimes.
    ///
    /// Returns data for each theater, including movies and screenings.
    async fn scrape(&self) -> Result<Vec<TheaterData>>;

    /// Optional: filter to specific cities.
    /// Default returns None (scrape all theaters).
    #[allow(dead_code)] // Kept for future multi-city filtering feature
    fn city_filter(&self) -> Option<&[&str]> {
        None
    }
}
