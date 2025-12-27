//! Scraper implementations for different cinema websites.

pub mod cinestar;
pub mod uci;
pub mod yorck;

pub use cinestar::CineStarScraper;
pub use uci::UciScraper;
pub use yorck::YorckScraper;
