//! Scraper implementations for different cinema websites.

pub mod cinestar;
pub mod critic;
pub mod yorck;

// UCI scraper disabled — Cloudflare blocks proxy/cloud IPs.
// UCI data is covered by critic.de scraper.
#[allow(dead_code)]
pub mod uci;

pub use cinestar::CineStarScraper;
pub use critic::CriticScraper;
pub use yorck::YorckScraper;
