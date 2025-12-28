//! UCI Kinowelt scraper.
//!
//! Scrapes movie showtimes from uci-kinowelt.de for Berlin theaters.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Europe::Berlin;
use reqwest::Client;
use scraper::{Html, Selector};
use std::collections::HashSet;

use crate::models::{Movie, MovieWithScreenings, Screening, Theater, TheaterData};
use crate::scraper::Scraper;

/// Berlin UCI theater locations with their IDs, addresses, and coordinates.
/// Format: (id, name, address, latitude, longitude)
const BERLIN_THEATERS: &[(&str, &str, &str, f64, f64)] = &[
    (
        "44",
        "UCI Berlin - Am Eastgate",
        "Marzahner Promenade 1, 12679 Berlin",
        52.5422,
        13.5444,
    ),
    (
        "82",
        "UCI Berlin - East Side Gallery | Luxe",
        "Mercedes-Platz 2, 10243 Berlin",
        52.5063,
        13.4517,
    ),
    (
        "43",
        "UCI Berlin - Gropius Passagen | Luxe",
        "Johannisthaler Chaussee 317, 12351 Berlin",
        52.4283,
        13.4572,
    ),
    (
        "59",
        "UCI Potsdam | Luxe",
        "Babelsberger Str. 10, 14473 Potsdam",
        52.3906,
        13.0647,
    ),
];

/// UCI Kinowelt website scraper.
pub struct UciScraper {
    client: Client,
    theater_ids: Vec<String>,
}

impl UciScraper {
    /// Creates a new UCI scraper for all Berlin theaters.
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36")
                .build()
                .expect("Failed to create HTTP client"),
            theater_ids: BERLIN_THEATERS
                .iter()
                .map(|(id, _, _, _, _)| id.to_string())
                .collect(),
        }
    }

    /// Scrapes a single theater's program.
    async fn scrape_theater(
        &self,
        theater_id: &str,
        theater_name: &str,
        address: &str,
        lat: f64,
        lng: f64,
    ) -> Result<TheaterData> {
        let slug = theater_name_to_slug(theater_name);
        let url = format!(
            "https://www.uci-kinowelt.de/kinoprogramm/{}/{}",
            slug, theater_id
        );

        println!("  Fetching: {}", url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch theater page")?;

        let html = response
            .text()
            .await
            .context("Failed to read response body")?;

        let slug = theater_name_to_slug(theater_name);
        let theater = Theater {
            external_id: theater_id.to_string(),
            name: theater_name.to_string(),
            city: extract_city(theater_name),
            address: Some(address.to_string()),
            url: Some(format!("https://www.uci-kinowelt.de/kinoprogramm/{}", slug)),
            latitude: Some(lat),
            longitude: Some(lng),
        };

        let movies = parse_theater_page(&html)?;

        Ok(TheaterData { theater, movies })
    }
}

impl Default for UciScraper {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Scraper for UciScraper {
    fn name(&self) -> &str {
        "uci_kinowelt"
    }

    fn url(&self) -> &str {
        "https://www.uci-kinowelt.de"
    }

    async fn scrape(&self) -> Result<Vec<TheaterData>> {
        let mut results = Vec::new();

        for (id, name, address, lat, lng) in BERLIN_THEATERS {
            if !self.theater_ids.is_empty() && !self.theater_ids.contains(&id.to_string()) {
                continue;
            }

            println!("Scraping: {}", name);
            match self.scrape_theater(id, name, address, *lat, *lng).await {
                Ok(data) => {
                    let screening_count: usize =
                        data.movies.iter().map(|m| m.screenings.len()).sum();
                    let ov_count: usize = data
                        .movies
                        .iter()
                        .flat_map(|m| &m.screenings)
                        .filter(|s| s.is_ov || s.is_omu || s.is_english_subs)
                        .count();
                    println!(
                        "  Found {} movies, {} screenings ({} OV/OmU)",
                        data.movies.len(),
                        screening_count,
                        ov_count
                    );
                    results.push(data);
                }
                Err(e) => {
                    eprintln!("  Error scraping {}: {}", name, e);
                }
            }
        }

        Ok(results)
    }
}

/// Converts theater name to URL slug.
fn theater_name_to_slug(name: &str) -> String {
    name.to_lowercase()
        .replace("uci ", "")
        .replace(" - ", "-")
        .replace(" | ", "-")
        .replace(' ', "-")
}

/// Extracts city from theater name.
fn extract_city(name: &str) -> Option<String> {
    if name.contains("Berlin") {
        Some("Berlin".to_string())
    } else if name.contains("Potsdam") {
        Some("Potsdam".to_string())
    } else {
        None
    }
}

/// Parses the theater program page HTML.
fn parse_theater_page(html: &str) -> Result<Vec<MovieWithScreenings>> {
    let document = Html::parse_document(html);
    let mut movies = Vec::new();
    let mut seen_urls: HashSet<String> = HashSet::new();

    // Film containers have class "film show"
    let film_selector = Selector::parse("div.film.show").unwrap();

    // Title selector - the main h2 with the title
    let title_selector =
        Selector::parse("h2.eventkalender--item--description--text--eventtitle a").unwrap();

    // Film info for runtime/rating
    let info_selector = Selector::parse("ul.film-info.infolist").unwrap();

    // Film ID from the eventkalender item
    let item_selector = Selector::parse("div.eventkalender--item").unwrap();

    // Performance/showtime links
    let performance_selector = Selector::parse("a.performance[href*='performanceId']").unwrap();

    // Schedule rows for date context
    let row_selector = Selector::parse("tr.schedule-container-date").unwrap();

    for film_elem in document.select(&film_selector) {
        // Get movie title
        let title = match film_elem.select(&title_selector).next() {
            Some(el) => el.text().collect::<String>().trim().to_string(),
            None => continue,
        };

        if title.is_empty() {
            continue;
        }

        // Get film ID
        let external_id = film_elem
            .select(&item_selector)
            .next()
            .and_then(|el| el.value().attr("film-id"))
            .map(|s| s.to_string());

        // Get runtime and rating from info list
        let (runtime, rating) = film_elem
            .select(&info_selector)
            .next()
            .map(|el| {
                let text = el.text().collect::<String>();
                parse_meta(&text)
            })
            .unwrap_or((None, None));

        // Check if the film section's legend mentions OV
        let film_html = film_elem.html();
        let film_section_has_ov = film_html.contains("OV: Filmvorstellung in Originalsprache");

        let movie = Movie {
            external_id,
            title: title.clone(),
            runtime_minutes: runtime,
            rating,
        };

        let mut screenings = Vec::new();

        // Parse all schedule rows
        for row in film_elem.select(&row_selector) {
            // Get date from row's data-date attribute (format: "20251227")
            let date = row
                .value()
                .attr("data-date")
                .and_then(|d| parse_date_compact(d));

            // Get all performances in this row
            for perf in row.select(&performance_selector) {
                let href = match perf.value().attr("href") {
                    Some(h) => h,
                    None => continue,
                };

                // Build full URL and check for duplicates
                let full_url = format!("https://www.uci-kinowelt.de{}", href);
                if seen_urls.contains(&full_url) {
                    continue;
                }
                seen_urls.insert(full_url.clone());

                // Get time from data-time attribute (format: "'17:15'")
                let time = perf
                    .value()
                    .attr("data-time")
                    .and_then(|t| parse_time_attr(t));

                let time = match time {
                    Some(t) => t,
                    None => continue,
                };

                // Get showtime date (from row or today)
                let showtime_date = date.unwrap_or_else(|| Utc::now().date_naive());

                // Combine into UTC datetime
                let showtime = Berlin
                    .from_local_datetime(&showtime_date.and_time(time))
                    .single()
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);

                // Check performance classes for attributes
                let class_attr = perf.value().attr("class").unwrap_or("");

                // OV detection: check if this specific performance has OV class
                let is_ov = class_attr.contains("attribut-ov") || class_attr.contains(" ov ");

                // OmU = German subtitles, OmeU/OmengU = English subtitles
                let is_omu = class_attr.contains("attribut-omu")
                    || (class_attr.contains(" omu ")
                        && !class_attr.contains("omeu")
                        && !class_attr.contains("omengu"));
                let is_english_subs = class_attr.contains("attribut-omeu")
                    || class_attr.contains(" omeu ")
                    || class_attr.contains("attribut-omengu")
                    || class_attr.contains(" omengu ");

                // 3D detection
                let is_3d = class_attr.contains("attribut-3d") || class_attr.contains(" 3d ");

                // Screening type detection
                let screening_type = detect_screening_type(class_attr);

                screenings.push(Screening {
                    showtime,
                    screening_type,
                    is_ov: is_ov
                        || (film_section_has_ov && !screenings.iter().any(|s: &Screening| s.is_ov)),
                    is_omu,
                    is_english_subs,
                    is_3d,
                    booking_url: Some(full_url),
                });
            }
        }

        if !screenings.is_empty() {
            movies.push(MovieWithScreenings { movie, screenings });
        }
    }

    Ok(movies)
}

/// Parses runtime and rating from info text.
fn parse_meta(text: &str) -> (Option<i32>, Option<String>) {
    let mut runtime = None;
    let mut rating = None;

    // Look for runtime like "193min"
    if let Some(caps) = regex::Regex::new(r"(\d+)\s*min")
        .ok()
        .and_then(|re| re.captures(text))
    {
        runtime = caps.get(1).and_then(|m| m.as_str().parse().ok());
    }

    // Look for FSK rating
    if let Some(caps) = regex::Regex::new(r"FSK\s*(\d+)")
        .ok()
        .and_then(|re| re.captures(text))
    {
        rating = caps.get(1).map(|m| format!("FSK {}", m.as_str()));
    }

    (runtime, rating)
}

/// Parses compact date format "20251227" -> NaiveDate
fn parse_date_compact(text: &str) -> Option<NaiveDate> {
    if text.len() != 8 {
        return None;
    }

    let year: i32 = text[0..4].parse().ok()?;
    let month: u32 = text[4..6].parse().ok()?;
    let day: u32 = text[6..8].parse().ok()?;

    NaiveDate::from_ymd_opt(year, month, day)
}

/// Parses time from data-time attribute like "'17:15'"
fn parse_time_attr(text: &str) -> Option<NaiveTime> {
    // Remove quotes: "'17:15'" -> "17:15"
    let cleaned = text.trim_matches(|c| c == '\'' || c == '"');

    NaiveTime::parse_from_str(cleaned, "%H:%M").ok()
}

/// Detects screening type from class attributes.
fn detect_screening_type(class_attr: &str) -> Option<String> {
    let mut types = Vec::new();

    if class_attr.contains("attribut-isense") || class_attr.contains(" isense ") {
        types.push("iSense");
    }
    if class_attr.contains("attribut-imax") || class_attr.contains(" imax") {
        types.push("IMAX");
    }
    if class_attr.contains("attribut-screenx") || class_attr.contains(" screenx ") {
        types.push("ScreenX");
    }
    if class_attr.contains("attribut-dolby") || class_attr.contains(" dolby ") {
        types.push("Dolby");
    }

    if types.is_empty() {
        None
    } else {
        Some(types.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_theater_name_to_slug() {
        assert_eq!(
            theater_name_to_slug("UCI Berlin - East Side Gallery | Luxe"),
            "berlin-east-side-gallery-luxe"
        );
    }

    #[test]
    fn test_parse_date_compact() {
        let date = parse_date_compact("20251227").unwrap();
        assert_eq!(date.year(), 2025);
        assert_eq!(date.month(), 12);
        assert_eq!(date.day(), 27);
    }

    #[test]
    fn test_parse_time_attr() {
        let time = parse_time_attr("'17:15'").unwrap();
        assert_eq!(time.hour(), 17);
        assert_eq!(time.minute(), 15);
    }

    #[test]
    fn test_parse_meta() {
        let (runtime, rating) = parse_meta("3. Spielwoche 193min FSK 12 Action");
        assert_eq!(runtime, Some(193));
        assert_eq!(rating, Some("FSK 12".to_string()));
    }
}
