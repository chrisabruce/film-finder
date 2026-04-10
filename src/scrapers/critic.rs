//! Critic.de OV Movies Berlin scraper.
//!
//! Scrapes movie showtimes from critic.de's OV (Original Version) movies section
//! for Berlin theaters. This source specializes in original language screenings.
//!
//! Note: This scraper only fetches OV/OmU screenings since that's what the
//! critic.de OV section provides. All screenings from this source are marked
//! as either OV or OmU based on the title tags.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Europe::Berlin;
use regex::Regex;
use reqwest::Client;
use scraper::{Html, Selector};

use crate::models::{Movie, MovieWithScreenings, Screening, Theater, TheaterData};
use crate::scraper::{http_client_builder, Scraper};

const BASE_URL: &str = "https://www.critic.de/ov-movies-berlin/";

/// Cinema info extracted from the main OV page.
#[derive(Debug, Clone)]
struct CinemaInfo {
    slug: String,
    name: String,
    address: String,
    postal_code: String,
    latitude: f64,
    longitude: f64,
}

/// Critic.de OV Movies scraper.
pub struct CriticScraper {
    client: Client,
}

impl CriticScraper {
    pub fn new() -> Self {
        Self {
            client: http_client_builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
                .build()
                .expect("Failed to create HTTP client"),
        }
    }

    /// Fetches the main OV page and extracts cinema information from JavaScript markers.
    async fn fetch_cinema_list(&self) -> Result<Vec<CinemaInfo>> {
        let response = self
            .client
            .get(BASE_URL)
            .send()
            .await
            .context("Failed to fetch OV movies main page")?;

        let html = response
            .text()
            .await
            .context("Failed to read response body")?;

        parse_cinema_list(&html)
    }

    /// Scrapes a single cinema's OV program.
    async fn scrape_cinema(&self, cinema: &CinemaInfo) -> Result<TheaterData> {
        let url = format!("{}cinema/{}/", BASE_URL, cinema.slug);
        println!("  Fetching: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch cinema page")?;

        let html = response
            .text()
            .await
            .context("Failed to read response body")?;

        let theater = Theater {
            external_id: cinema.slug.clone(),
            name: cinema.name.clone(),
            city: Some("Berlin".to_string()),
            address: Some(format!("{}, {} Berlin", cinema.address, cinema.postal_code)),
            url: Some(url),
            latitude: Some(cinema.latitude),
            longitude: Some(cinema.longitude),
        };

        let movies = parse_cinema_page(&html)?;

        Ok(TheaterData { theater, movies })
    }
}

impl Default for CriticScraper {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Scraper for CriticScraper {
    fn name(&self) -> &str {
        "critic_de"
    }

    fn url(&self) -> &str {
        "https://www.critic.de/ov-movies-berlin/"
    }

    async fn scrape(&self) -> Result<Vec<TheaterData>> {
        println!("  Fetching cinema list...");
        let cinemas = self.fetch_cinema_list().await?;
        println!("  Found {} cinemas with OV screenings", cinemas.len());

        let mut results = Vec::new();

        for cinema in &cinemas {
            match self.scrape_cinema(cinema).await {
                Ok(data) => {
                    if !data.movies.is_empty() {
                        let screening_count: usize =
                            data.movies.iter().map(|m| m.screenings.len()).sum();
                        println!(
                            "  {} - {} movies, {} screenings",
                            cinema.name,
                            data.movies.len(),
                            screening_count
                        );
                        results.push(data);
                    }
                }
                Err(e) => {
                    eprintln!("  Error scraping {}: {}", cinema.name, e);
                }
            }
        }

        Ok(results)
    }
}

/// Parses the main OV page to extract cinema information from JavaScript addMarker calls.
fn parse_cinema_list(html: &str) -> Result<Vec<CinemaInfo>> {
    let mut cinemas = Vec::new();

    // Pattern: addMarker(false,"52.5043600","13.3195000","Filmkunst 66","Bleibtreustraße 12","10623","Berlin","https://www.critic.de/ov-movies-berlin/cinema/filmkunst-66/")
    let marker_regex = Regex::new(
        r#"addMarker\(false,"([^"]+)","([^"]+)","([^"]+)","([^"]+)","([^"]+)","Berlin","https://www\.critic\.de/ov-movies-berlin/cinema/([^/]+)/"\)"#,
    )?;

    for cap in marker_regex.captures_iter(html) {
        let latitude: f64 = cap[1].parse().unwrap_or(0.0);
        let longitude: f64 = cap[2].parse().unwrap_or(0.0);
        let name = cap[3].to_string();
        let address = cap[4].to_string();
        let postal_code = cap[5].to_string();
        let slug = cap[6].to_string();

        if latitude != 0.0 && longitude != 0.0 {
            cinemas.push(CinemaInfo {
                slug,
                name,
                address,
                postal_code,
                latitude,
                longitude,
            });
        }
    }

    Ok(cinemas)
}

/// Parses a cinema's OV program page.
fn parse_cinema_page(html: &str) -> Result<Vec<MovieWithScreenings>> {
    let document = Html::parse_document(html);
    let mut movies = Vec::new();

    // Find the films section
    let article_selector = Selector::parse("section#filme article").unwrap();
    let title_selector = Selector::parse("h3 a").unwrap();
    let table_selector = Selector::parse("table.vorstellung").unwrap();
    let thead_selector = Selector::parse("thead tr th").unwrap();
    let tbody_selector = Selector::parse("tbody tr").unwrap();
    let td_selector = Selector::parse("td").unwrap();

    // Extract date headers from the page to map column indices to dates
    // We need to find the current date context from the page

    for article in document.select(&article_selector) {
        // Get movie title from h3 a
        let title_elem = match article.select(&title_selector).next() {
            Some(el) => el,
            None => continue,
        };

        let raw_title = title_elem.text().collect::<String>().trim().to_string();
        if raw_title.is_empty() {
            continue;
        }

        // Parse language info from title and clean it
        let (clean_title, is_ov, is_omu, is_english_subs) = parse_title_language(&raw_title);

        // Extract movie ID from article class (e.g., "highlight_movie_413622")
        let external_id = article.value().attr("class").and_then(|c| {
            c.split_whitespace()
                .find(|s| s.starts_with("highlight_movie_"))
                .map(|s| s.strip_prefix("highlight_movie_").unwrap_or(s).to_string())
        });

        let movie = Movie {
            external_id,
            title: clean_title,
            runtime_minutes: None,
            rating: None,
        };

        let mut screenings = Vec::new();

        // Parse the showtime table
        if let Some(table) = article.select(&table_selector).next() {
            // Get date headers
            let headers: Vec<String> = table
                .select(&thead_selector)
                .map(|th| th.text().collect::<String>().trim().to_string())
                .collect();

            // Parse dates from headers (format: "Today", "Thu 08/01", "Fri 09/01", etc.)
            let dates = parse_date_headers(&headers);

            // Parse showtime rows
            for row in table.select(&tbody_selector) {
                let cells: Vec<_> = row.select(&td_selector).collect();

                for (col_idx, cell) in cells.iter().enumerate() {
                    if col_idx >= dates.len() {
                        continue;
                    }

                    let date = match &dates[col_idx] {
                        Some(d) => *d,
                        None => continue,
                    };

                    // Get all times from this cell (can have multiple separated by <br>)
                    let cell_text = cell.text().collect::<String>();
                    let times: Vec<&str> = cell_text
                        .split('\n')
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect();

                    for time_str in times {
                        if let Some(time) = parse_time(time_str) {
                            let showtime = Berlin
                                .from_local_datetime(&date.and_time(time))
                                .single()
                                .map(|dt| dt.with_timezone(&Utc));

                            if let Some(showtime) = showtime {
                                screenings.push(Screening {
                                    showtime,
                                    screening_type: None,
                                    is_ov,
                                    is_omu,
                                    is_english_subs,
                                    is_3d: raw_title.contains("3D"),
                                    booking_url: None,
                                });
                            }
                        }
                    }
                }
            }
        }

        if !screenings.is_empty() {
            movies.push(MovieWithScreenings { movie, screenings });
        }
    }

    Ok(movies)
}

/// Parses the movie title to extract language version info and return a clean title.
/// Returns (clean_title, is_ov, is_omu, is_english_subs)
fn parse_title_language(title: &str) -> (String, bool, bool, bool) {
    let title_lower = title.to_lowercase();

    // Check for language indicators
    // OV w/ sub or OmU = original with (German) subtitles
    // OmeU or OmengU = original with English subtitles
    // OF or OV = original version (no subtitles specified)

    let is_english_subs = title_lower.contains("omeu")
        || title_lower.contains("omengu")
        || title_lower.contains("english sub");

    let is_omu =
        (title_lower.contains("omu") || title_lower.contains("ov w/ sub")) && !is_english_subs;

    // OF (Originalfassung) or OV without subtitle specifier
    let is_ov = (title_lower.contains("(of)") || title_lower.contains("(ov)"))
        && !is_omu
        && !is_english_subs;

    // If from the OV section but no specific marker, assume it's at least OV
    let is_ov = is_ov
        || (!is_omu
            && !is_english_subs
            && (title_lower.contains("ov") || title_lower.contains("of")));

    // Clean the title by removing language tags
    let clean_title = title
        .replace("(OV w/ sub)", "")
        .replace("(OV w/sub)", "")
        .replace("(OmU)", "")
        .replace("(OmeU)", "")
        .replace("(OmengU)", "")
        .replace("(OF)", "")
        .replace("(OV)", "")
        .replace("(HFR 3D)", "")
        .replace("(HFR)", "")
        .replace("(3D)", "")
        .trim()
        .to_string();

    (clean_title, is_ov, is_omu, is_english_subs)
}

/// Parses date headers like "Today", "Thu 08/01", "Fri 09/01" into NaiveDates.
fn parse_date_headers(headers: &[String]) -> Vec<Option<NaiveDate>> {
    let today = Utc::now().with_timezone(&Berlin).date_naive();
    let current_year = today.year();

    headers
        .iter()
        .enumerate()
        .map(|(idx, header)| {
            let header_lower = header.to_lowercase();

            // Handle "Today" / "Heute"
            if header_lower.contains("today") || header_lower.contains("heute") {
                return Some(today);
            }

            // Handle format like "Thu 08/01" or "Do, 08.01."
            // Try to extract day and month
            let date_regex = Regex::new(r"(\d{1,2})[/.](\d{1,2})").ok()?;

            if let Some(caps) = date_regex.captures(header) {
                let day: u32 = caps[1].parse().ok()?;
                let month: u32 = caps[2].parse().ok()?;

                // Determine year - if the date seems to be in the past, it might be next year
                let mut year = current_year;
                if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
                    if date < today && (today - date).num_days() > 180 {
                        year += 1;
                    }
                    return NaiveDate::from_ymd_opt(year, month, day);
                }
            }

            // Fallback: assume it's today + index days
            Some(today + chrono::Duration::days(idx as i64))
        })
        .collect()
}

/// Parses a time string like "15:00" or "15.00" into NaiveTime.
fn parse_time(time_str: &str) -> Option<NaiveTime> {
    let cleaned = time_str.trim().replace('.', ":");

    // Try HH:MM format
    if let Ok(time) = NaiveTime::parse_from_str(&cleaned, "%H:%M") {
        return Some(time);
    }

    // Try H:MM format
    if let Ok(time) = NaiveTime::parse_from_str(&cleaned, "%-H:%M") {
        return Some(time);
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_title_language() {
        let (title, is_ov, is_omu, is_english_subs) =
            parse_title_language("L'Étranger (OV w/ sub)");
        assert_eq!(title, "L'Étranger");
        assert!(!is_ov);
        assert!(is_omu);
        assert!(!is_english_subs);

        let (title, is_ov, is_omu, is_english_subs) = parse_title_language("Hook (OF)");
        assert_eq!(title, "Hook");
        assert!(is_ov);
        assert!(!is_omu);
        assert!(!is_english_subs);

        let (title, is_ov, is_omu, is_english_subs) = parse_title_language("Some Movie (OmeU)");
        assert_eq!(title, "Some Movie");
        assert!(!is_ov);
        assert!(!is_omu);
        assert!(is_english_subs);
    }

    #[test]
    fn test_parse_time() {
        assert_eq!(
            parse_time("15:00"),
            Some(NaiveTime::from_hms_opt(15, 0, 0).unwrap())
        );
        assert_eq!(
            parse_time("9:30"),
            Some(NaiveTime::from_hms_opt(9, 30, 0).unwrap())
        );
        assert_eq!(
            parse_time("15.00"),
            Some(NaiveTime::from_hms_opt(15, 0, 0).unwrap())
        );
    }
}
