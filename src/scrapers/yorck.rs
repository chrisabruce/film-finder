//! Yorck Kino scraper.
//!
//! Fetches showtime data from Yorck's Next.js page data for Berlin theaters.

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Europe::Berlin;
use serde::Deserialize;
use std::collections::HashMap;

use crate::models::{Movie, MovieWithScreenings, Screening, Theater, TheaterData};
use crate::scraper::{http_client_builder, Scraper};

const FILMS_URL: &str = "https://www.yorck.de/en/films";
const CINEMAS_URL: &str = "https://www.yorck.de/en/cinemas";

/// Yorck scraper implementation.
pub struct YorckScraper {
    client: reqwest::Client,
}

impl YorckScraper {
    pub fn new() -> Self {
        Self {
            client: http_client_builder()
                .build()
                .expect("Failed to create HTTP client"),
        }
    }
}

#[async_trait]
impl Scraper for YorckScraper {
    fn name(&self) -> &str {
        "Yorck"
    }

    fn url(&self) -> &str {
        "https://www.yorck.de"
    }

    async fn scrape(&self) -> Result<Vec<TheaterData>> {
        // First fetch cinema info for addresses
        println!("  Fetching cinemas page...");
        let cinemas_html = self.client.get(CINEMAS_URL).send().await?.text().await?;
        let cinemas_json = extract_next_data(&cinemas_html)?;
        let cinemas_data: CinemasNextData = serde_json::from_str(&cinemas_json)?;

        // Build a map of cinema name -> (address, url, lat, lng)
        let mut cinema_info: HashMap<String, (String, Option<String>, Option<f64>, Option<f64>)> =
            HashMap::new();
        for cinema in cinemas_data.props.page_props.cinemas {
            let coords = cinema.fields.coordinates;
            let url = cinema
                .fields
                .slug
                .map(|slug| format!("https://www.yorck.de/en/cinemas/{}", slug));
            cinema_info.insert(
                cinema.fields.name.clone(),
                (
                    cinema.fields.address,
                    url,
                    coords.as_ref().map(|c| c.lat),
                    coords.as_ref().map(|c| c.lon),
                ),
            );
        }

        println!("  Fetching films page...");
        let html = self.client.get(FILMS_URL).send().await?.text().await?;

        // Extract __NEXT_DATA__ JSON from the page
        let json_str = extract_next_data(&html)?;
        let data: NextData = serde_json::from_str(&json_str)?;

        // Group sessions by theater
        let mut theaters_map: HashMap<String, TheaterData> = HashMap::new();

        for film in data.props.page_props.films {
            for session in film.fields.sessions {
                let cinema_name = session.fields.cinema.fields.name.clone();

                let (address, url, lat, lng) = cinema_info
                    .get(&cinema_name)
                    .cloned()
                    .unwrap_or_else(|| (String::new(), None, None, None));

                let theater_data =
                    theaters_map
                        .entry(cinema_name.clone())
                        .or_insert_with(|| TheaterData {
                            theater: Theater {
                                external_id: cinema_name.clone(),
                                name: cinema_name.clone(),
                                city: Some("Berlin".to_string()),
                                address: if address.is_empty() {
                                    None
                                } else {
                                    Some(address)
                                },
                                url,
                                latitude: lat,
                                longitude: lng,
                            },
                            movies: Vec::new(),
                        });

                // Find or create movie entry
                let movie_entry = theater_data
                    .movies
                    .iter_mut()
                    .find(|m| m.movie.external_id.as_ref() == Some(&film.sys.id));

                let screening = parse_session(&session)?;

                if let Some(entry) = movie_entry {
                    entry.screenings.push(screening);
                } else {
                    theater_data.movies.push(MovieWithScreenings {
                        movie: Movie {
                            external_id: Some(film.sys.id.clone()),
                            title: film.fields.title.clone(),
                            runtime_minutes: film.fields.runtime,
                            rating: film.fields.fsk.map(|f| format!("FSK {}", f)),
                        },
                        screenings: vec![screening],
                    });
                }
            }
        }

        let results: Vec<TheaterData> = theaters_map.into_values().collect();

        let total_movies: usize = results.iter().map(|t| t.movies.len()).sum();
        println!(
            "  Found {} theaters, {} movies",
            results.len(),
            total_movies
        );

        Ok(results)
    }
}

fn extract_next_data(html: &str) -> Result<String> {
    let start_marker = r#"__NEXT_DATA__" type="application/json">"#;
    let start = html
        .find(start_marker)
        .ok_or_else(|| anyhow!("Could not find __NEXT_DATA__ in page"))?;
    let json_start = start + start_marker.len();

    let end_marker = "</script>";
    let end = html[json_start..]
        .find(end_marker)
        .ok_or_else(|| anyhow!("Could not find end of __NEXT_DATA__"))?;

    Ok(html[json_start..json_start + end].to_string())
}

fn parse_session(session: &Session) -> Result<Screening> {
    // The Yorck API consistently sends times with +01:00 (CET) offset even during
    // CEST (summer time, +02:00). Trusting the offset would store times 1 hour
    // early in UTC, causing them to display 1 hour late in Berlin.
    // Fix: strip the offset and re-interpret the wall-clock time as Berlin local time
    // so chrono_tz correctly applies CET in winter and CEST in summer.
    let naive = DateTime::parse_from_rfc3339(&session.fields.start_time)
        .map_err(|e| {
            anyhow!(
                "Failed to parse showtime '{}': {}",
                &session.fields.start_time,
                e
            )
        })?
        .naive_local();

    let showtime: DateTime<Utc> = Berlin
        .from_local_datetime(&naive)
        .single()
        .ok_or_else(|| {
            anyhow!(
                "Ambiguous or non-existent local time: {}",
                &session.fields.start_time
            )
        })?
        .with_timezone(&Utc);

    let formats = &session.fields.formats;

    // Check for OV/OmU in formats
    let is_ov = formats.iter().any(|f| f == "OV");
    // OmU = German subtitles
    let is_omu = formats.iter().any(|f| f == "OmU");
    // OmeU/OmengU = English subtitles
    let is_english_subs = formats.iter().any(|f| f == "OmeU" || f == "OmengU");
    let is_3d = formats.iter().any(|f| f.contains("3D"));

    // Get special format if present
    let screening_type = formats
        .iter()
        .find(|f| f.contains("Dolby") || f.contains("IMAX") || f.contains("HFR"))
        .cloned();

    Ok(Screening {
        showtime,
        screening_type,
        is_ov,
        is_omu,
        is_english_subs,
        is_3d,
        booking_url: None,
    })
}

// Serde structs for parsing Next.js data

// Cinemas page structures
#[derive(Debug, Deserialize)]
struct CinemasNextData {
    props: CinemasProps,
}

#[derive(Debug, Deserialize)]
struct CinemasProps {
    #[serde(rename = "pageProps")]
    page_props: CinemasPageProps,
}

#[derive(Debug, Deserialize)]
struct CinemasPageProps {
    cinemas: Vec<CinemaEntry>,
}

#[derive(Debug, Deserialize)]
struct CinemaEntry {
    fields: CinemaEntryFields,
}

#[derive(Debug, Deserialize)]
struct CinemaEntryFields {
    name: String,
    slug: Option<String>,
    #[serde(default)]
    address: String,
    coordinates: Option<Coordinates>,
}

#[derive(Debug, Deserialize, Clone)]
struct Coordinates {
    lat: f64,
    lon: f64,
}

// Films page structures
#[derive(Debug, Deserialize)]
struct NextData {
    props: Props,
}

#[derive(Debug, Deserialize)]
struct Props {
    #[serde(rename = "pageProps")]
    page_props: PageProps,
}

#[derive(Debug, Deserialize)]
struct PageProps {
    films: Vec<Film>,
}

#[derive(Debug, Deserialize)]
struct Film {
    sys: Sys,
    fields: FilmFields,
}

#[derive(Debug, Deserialize)]
struct Sys {
    id: String,
}

#[derive(Debug, Deserialize)]
struct FilmFields {
    title: String,
    runtime: Option<i32>,
    fsk: Option<i32>,
    #[serde(default)]
    sessions: Vec<Session>,
}

#[derive(Debug, Deserialize)]
struct Session {
    fields: SessionFields,
}

#[derive(Debug, Deserialize)]
struct SessionFields {
    #[serde(rename = "startTime")]
    start_time: String,
    #[serde(default)]
    formats: Vec<String>,
    cinema: Cinema,
}

#[derive(Debug, Deserialize)]
struct Cinema {
    fields: CinemaFields,
}

#[derive(Debug, Deserialize)]
struct CinemaFields {
    name: String,
}
