//! Scraper trait for extensibility.
//!
//! Implement this trait to add support for new cinema websites.

use anyhow::Result;
use async_trait::async_trait;

use crate::models::TheaterData;

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
