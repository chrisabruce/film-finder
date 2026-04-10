//! CineStar cinema scraper.
//!
//! Fetches showtime data from CineStar's JSON API for Berlin theaters.

use anyhow::Result;
use async_trait::async_trait;
use chrono::{NaiveDateTime, TimeZone, Utc};
use chrono_tz::Europe::Berlin;
use serde::Deserialize;

use crate::models::{Movie, MovieWithScreenings, Screening, Theater, TheaterData};
use crate::scraper::{http_client_builder, Scraper};

/// Berlin CineStar theater IDs and names.
const BERLIN_THEATERS: &[(&str, &str)] = &[
    ("3", "CineStar Berlin - CUBIX am Alexanderplatz"),
    ("5", "CineStar Berlin - Tegel"),
    ("6", "CineStar Berlin - Hellersdorf"),
    ("9", "Kino in der KulturBrauerei"),
];

const API_BASE: &str = "https://www.cinestar.de/api/cinema";

/// CineStar scraper implementation.
pub struct CineStarScraper {
    client: reqwest::Client,
}

impl CineStarScraper {
    pub fn new() -> Self {
        Self {
            client: http_client_builder()
                .build()
                .expect("Failed to create HTTP client"),
        }
    }
}

#[async_trait]
impl Scraper for CineStarScraper {
    fn name(&self) -> &str {
        "CineStar"
    }

    fn url(&self) -> &str {
        "https://www.cinestar.de"
    }

    async fn scrape(&self) -> Result<Vec<TheaterData>> {
        let mut results = Vec::new();

        for (theater_id, theater_name) in BERLIN_THEATERS {
            print!("  Fetching {}... ", theater_name);

            match self.scrape_theater(theater_id).await {
                Ok(data) => {
                    println!("{} movies", data.movies.len());
                    results.push(data);
                }
                Err(e) => {
                    println!("error: {}", e);
                }
            }
        }

        Ok(results)
    }
}

impl CineStarScraper {
    async fn fetch_cinema_info(&self, theater_id: &str) -> Result<CinemaInfo> {
        let url = format!("{}/{}/", API_BASE, theater_id);
        let info: CinemaInfo = self.client.get(&url).send().await?.json().await?;
        Ok(info)
    }

    async fn scrape_theater(&self, theater_id: &str) -> Result<TheaterData> {
        // Fetch cinema info for address and coordinates
        let cinema_info = self.fetch_cinema_info(theater_id).await?;

        let url = format!("{}/{}/show/", API_BASE, theater_id);
        let movies_response: Vec<MovieEntry> = self.client.get(&url).send().await?.json().await?;

        let address = format!(
            "{}, {} {}",
            cinema_info.street, cinema_info.zip, cinema_info.city
        );

        let theater_url = cinema_info
            .slug
            .as_ref()
            .map(|slug| format!("https://www.cinestar.de/kino/{}", slug));

        let theater = Theater {
            external_id: theater_id.to_string(),
            name: cinema_info.name,
            city: Some(cinema_info.city),
            address: Some(address),
            url: theater_url,
            latitude: cinema_info.lat,
            longitude: cinema_info.lng,
        };

        let mut movies = Vec::new();

        for movie_entry in movies_response {
            let mut screenings = Vec::new();

            for showtime in movie_entry.showtimes {
                // Parse datetime like "2025-12-27 16:00 CET"
                if let Some(utc) = parse_cinestar_datetime(&showtime.datetime) {
                    // Check for OV attributes
                    let is_ov = showtime
                        .attributes
                        .iter()
                        .any(|a| a == "OV" || a == "AUDIO_OV");
                    // OmU = German subtitles
                    let is_omu = showtime.attributes.iter().any(|a| a == "OmU");
                    // OmeU/OmengU = English subtitles
                    let is_english_subs = showtime
                        .attributes
                        .iter()
                        .any(|a| a == "OmeU" || a == "OmengU");
                    let is_3d = showtime.attributes.iter().any(|a| a.contains("3D"));

                    // Format type from attributes
                    let screening_type = showtime
                        .attributes
                        .iter()
                        .find(|a| {
                            a.contains("IMAX")
                                || a.contains("Dolby")
                                || a.contains("Premium")
                                || a.contains("XL")
                                || a.contains("HFR")
                        })
                        .cloned();

                    screenings.push(Screening {
                        showtime: utc,
                        screening_type,
                        is_ov,
                        is_omu,
                        is_english_subs,
                        is_3d,
                        booking_url: Some(format!(
                            "https://www.cinestar.de/tickets/{}",
                            showtime.id
                        )),
                    });
                }
            }

            if !screenings.is_empty() {
                movies.push(MovieWithScreenings {
                    movie: Movie {
                        external_id: Some(movie_entry.id.to_string()),
                        title: movie_entry.title,
                        runtime_minutes: None,
                        rating: None,
                    },
                    screenings,
                });
            }
        }

        Ok(TheaterData { theater, movies })
    }
}

/// Parses CineStar datetime format like "2025-12-27 16:00 CET"
fn parse_cinestar_datetime(s: &str) -> Option<chrono::DateTime<Utc>> {
    // Remove timezone suffix and parse as naive datetime
    let parts: Vec<&str> = s.rsplitn(2, ' ').collect();
    if parts.len() < 2 {
        return None;
    }
    let datetime_str = parts[1];

    NaiveDateTime::parse_from_str(datetime_str, "%Y-%m-%d %H:%M")
        .ok()
        .and_then(|naive| Berlin.from_local_datetime(&naive).single())
        .map(|local| local.with_timezone(&Utc))
}

#[derive(Debug, Deserialize)]
struct CinemaInfo {
    name: String,
    slug: Option<String>,
    city: String,
    street: String,
    zip: String,
    lat: Option<f64>,
    lng: Option<f64>,
}

/// Movie entry from the API (array of movies with nested showtimes)
#[derive(Debug, Deserialize)]
struct MovieEntry {
    id: i64,
    title: String,
    #[serde(default)]
    showtimes: Vec<ShowtimeEntry>,
}

#[derive(Debug, Deserialize)]
struct ShowtimeEntry {
    id: i64,
    datetime: String,
    #[serde(default)]
    attributes: Vec<String>,
}
