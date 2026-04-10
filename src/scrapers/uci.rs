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
use std::env;

/// Returns true if DEBUG_SCRAPERS env var is set to "true"
fn debug_enabled() -> bool {
    env::var("DEBUG_SCRAPERS")
        .map(|v| v.to_lowercase() == "true")
        .unwrap_or(false)
}

use crate::models::{Movie, MovieWithScreenings, Screening, Theater, TheaterData};
use crate::scraper::{http_client_builder, Scraper};

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
        use reqwest::header::{
            HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, CACHE_CONTROL, CONNECTION,
        };

        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8"));
        headers.insert(
            ACCEPT_LANGUAGE,
            HeaderValue::from_static("en-US,en;q=0.9,de;q=0.8"),
        );
        headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-cache"));
        headers.insert(CONNECTION, HeaderValue::from_static("keep-alive"));
        headers.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
        headers.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
        headers.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
        headers.insert("Sec-Fetch-User", HeaderValue::from_static("?1"));
        headers.insert("Upgrade-Insecure-Requests", HeaderValue::from_static("1"));

        Self {
            client: http_client_builder()
                .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
                .default_headers(headers)
                .cookie_store(true)
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
    let debug = debug_enabled();

    if debug {
        eprintln!("[DEBUG] HTML size: {} bytes", html.len());
    }

    // Film containers: <div data-film-container data-film-id="...">
    let film_selector = Selector::parse("div[data-film-container]").unwrap();

    // Title: <h2 class="film-container__description__text__eventtitle"> <a>Title</a> </h2>
    let title_selector =
        Selector::parse("h2.film-container__description__text__eventtitle a").unwrap();

    // Film info for runtime/rating
    let info_selector = Selector::parse("ul.film-info.infolist").unwrap();

    // Performance/showtime links: <a class="badge-performance" data-time="16:30" data-date="20260403" href="...">
    let performance_selector = Selector::parse("a.badge-performance").unwrap();

    let film_elements: Vec<_> = document.select(&film_selector).collect();
    if debug {
        eprintln!(
            "[DEBUG] Found {} elements matching 'div[data-film-container]'",
            film_elements.len()
        );
    }

    let mut titles_without_screenings = 0;
    let mut date_parse_failures = 0;
    let mut time_parse_failures = 0;

    for film_elem in film_elements {
        // Get movie title
        let title = match film_elem.select(&title_selector).next() {
            Some(el) => el.text().collect::<String>().trim().to_string(),
            None => {
                if debug {
                    eprintln!("[DEBUG] Film element missing title");
                }
                continue;
            }
        };

        if title.is_empty() {
            continue;
        }

        // Get film ID from data-film-id attribute
        let external_id = film_elem
            .value()
            .attr("data-film-id")
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
        let film_section_has_ov = film_html.contains("data-attribute-id=\"ov\"");

        let movie = Movie {
            external_id,
            title: title.clone(),
            runtime_minutes: runtime,
            rating,
        };

        let mut screenings = Vec::new();

        // Parse all performance badges directly (they carry data-date and data-time)
        let performances: Vec<_> = film_elem.select(&performance_selector).collect();
        if debug && performances.is_empty() {
            eprintln!(
                "[DEBUG] Movie '{}' has no performance badges",
                title
            );
        }

        for perf in performances {
            let href = match perf.value().attr("href") {
                Some(h) => h,
                None => continue,
            };

            let full_url = format!("https://www.uci-kinowelt.de{}", href);
            if seen_urls.contains(&full_url) {
                continue;
            }
            seen_urls.insert(full_url.clone());

            // Get time from data-time attribute (format: "16:30")
            let time_attr = perf.value().attr("data-time");
            let time = time_attr.and_then(|t| parse_time_attr(t));

            let time = match time {
                Some(t) => t,
                None => {
                    if debug {
                        time_parse_failures += 1;
                        eprintln!(
                            "[DEBUG] Failed to parse time '{}' for movie '{}'",
                            time_attr.unwrap_or("(missing)"),
                            title
                        );
                    }
                    continue;
                }
            };

            // Get date from data-date attribute (format: "20260403")
            let date_attr = perf.value().attr("data-date");
            let date = date_attr.and_then(|d| parse_date_compact(d));

            if debug && date_attr.is_some() && date.is_none() {
                date_parse_failures += 1;
                eprintln!(
                    "[DEBUG] Failed to parse date '{}' for movie '{}'",
                    date_attr.unwrap(),
                    title
                );
            }

            let showtime_date = date.unwrap_or_else(|| Utc::now().date_naive());

            let showtime = Berlin
                .from_local_datetime(&showtime_date.and_time(time))
                .single()
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);

            // Check performance classes for attributes
            let class_attr = perf.value().attr("class").unwrap_or("");

            let is_ov = class_attr.contains("attribute-ov");
            let is_omu = class_attr.contains("attribute-omu")
                && !class_attr.contains("attribute-omeu")
                && !class_attr.contains("attribute-omengu");
            let is_english_subs = class_attr.contains("attribute-omeu")
                || class_attr.contains("attribute-omengu");
            let is_3d = class_attr.contains("attribute-3d");
            let screening_type = detect_screening_type(class_attr);

            screenings.push(Screening {
                showtime,
                screening_type,
                is_ov: is_ov || (film_section_has_ov && is_section_ov_default(&film_html)),
                is_omu,
                is_english_subs,
                is_3d,
                booking_url: Some(full_url),
            });
        }

        if !screenings.is_empty() {
            movies.push(MovieWithScreenings { movie, screenings });
        } else {
            titles_without_screenings += 1;
            if debug {
                eprintln!(
                    "[DEBUG] Movie '{}' has no valid screenings, skipping",
                    title
                );
            }
        }
    }

    if debug {
        eprintln!("[DEBUG] --- Parsing Summary ---");
        eprintln!("[DEBUG] Movies with screenings: {}", movies.len());
        eprintln!(
            "[DEBUG] Movies skipped (no screenings): {}",
            titles_without_screenings
        );
        eprintln!("[DEBUG] Date parse failures: {}", date_parse_failures);
        eprintln!("[DEBUG] Time parse failures: {}", time_parse_failures);
        eprintln!("[DEBUG] --------------------------");
    }

    Ok(movies)
}

/// Returns true if the film section legend lists OV but individual badges
/// don't carry attribute-ov (i.e. all screenings in this section are OV).
fn is_section_ov_default(film_html: &str) -> bool {
    // If the legend declares OV but there are no individual attribute-ov badges,
    // all performances in this section are OV.
    film_html.contains("data-attribute-id=\"ov\"")
        && !film_html.contains("attribute-ov")
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

    if class_attr.contains("attribute-isense") {
        types.push("iSense");
    }
    if class_attr.contains("attribute-imax") {
        types.push("IMAX");
    }
    if class_attr.contains("attribute-screenx") {
        types.push("ScreenX");
    }
    if class_attr.contains("attribute-dolby") {
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
    use chrono::{Datelike, Timelike};

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
