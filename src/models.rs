//! Data models shared across scrapers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A cinema/theater location.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theater {
    /// ID from the source website
    pub external_id: String,
    /// Display name
    pub name: String,
    /// City (e.g., "Berlin")
    pub city: Option<String>,
    /// Street address
    pub address: Option<String>,
    /// Theater website URL
    pub url: Option<String>,
    /// Latitude for proximity search
    pub latitude: Option<f64>,
    /// Longitude for proximity search
    pub longitude: Option<f64>,
}

/// A movie.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Movie {
    /// ID from the source website
    pub external_id: Option<String>,
    /// Movie title
    pub title: String,
    /// Runtime in minutes
    pub runtime_minutes: Option<i32>,
    /// Age rating (FSK in Germany)
    pub rating: Option<String>,
}

/// A single screening of a movie.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Screening {
    /// When the movie plays
    pub showtime: DateTime<Utc>,
    /// Format info (iSense, IMAX, etc.)
    pub screening_type: Option<String>,
    /// Original Version (no dubbing)
    pub is_ov: bool,
    /// Original with German subtitles (OmU)
    pub is_omu: bool,
    /// Original with English subtitles (OmeU/OmengU)
    pub is_english_subs: bool,
    /// 3D screening
    pub is_3d: bool,
    /// Direct booking link
    pub booking_url: Option<String>,
}

/// Data scraped for a single theater.
#[derive(Debug, Clone)]
pub struct TheaterData {
    pub theater: Theater,
    pub movies: Vec<MovieWithScreenings>,
}

/// A movie with its screenings at a specific theater.
#[derive(Debug, Clone)]
pub struct MovieWithScreenings {
    pub movie: Movie,
    pub screenings: Vec<Screening>,
}
