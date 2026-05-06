//! Static website generator for browsing OV/OmU movies.
//!
//! Generates a clean, dark-themed static HTML site with movie posters,
//! descriptions, and showtimes filtered for English-language screenings.

use anyhow::Result;
use chrono::{DateTime, Utc};
use chrono_tz::Europe::Berlin;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use crate::db::Database;

/// Movie data for the static site.
#[derive(Debug)]
#[allow(dead_code)]
struct MovieData {
    id: i64,
    title: String,
    english_title: Option<String>,
    original_title: Option<String>,
    german_title: Option<String>,
    original_language: Option<String>,
    production_countries: Option<String>,
    year: Option<i32>,
    runtime_minutes: Option<i32>,
    genres: Option<String>,
    overview: Option<String>,
    poster_url: Option<String>,
    tmdb_url: Option<String>,
    director: Option<String>,
    director_id: Option<i32>,
    writer: Option<String>,
    writer_id: Option<i32>,
    cinematographer: Option<String>,
    cinematographer_id: Option<i32>,
    screenings: Vec<ScreeningData>,
}

/// Screening data for the static site.
#[derive(Debug)]
struct ScreeningData {
    theater_name: String,
    normalized_theater_name: String,
    theater_url: Option<String>,
    showtime: DateTime<Utc>,
    is_ov: bool,
    is_omu: bool,
    is_english_subs: bool,
    is_3d: bool,
    screening_type: Option<String>,
    booking_url: Option<String>,
}

/// Theater data for filtering (deduplicated).
#[derive(Debug, Clone)]
struct TheaterInfo {
    /// Normalized name used as the unique key
    normalized_name: String,
    /// Display name (best version from sources)
    display_name: String,
    /// URL to theater website
    url: Option<String>,
    /// Theater chain or group
    chain: String,
    /// All database IDs that map to this theater
    db_ids: Vec<i64>,
}

/// Generates the static website.
pub fn generate_static_site(db: &Database, output_dir: &str) -> Result<()> {
    let output_path = Path::new(output_dir);

    // Create output directory if it doesn't exist
    fs::create_dir_all(output_path)?;

    // Fetch all data
    let theaters = fetch_theaters(db)?;
    let movies = fetch_ov_movies(db)?;

    // Generate cache-busting version string (Unix timestamp)
    let cache_version = Utc::now().timestamp();

    // Generate HTML
    let html = generate_html(&movies, &theaters, cache_version);

    // Write files
    fs::write(output_path.join("index.html"), html)?;
    fs::write(output_path.join("style.css"), generate_css())?;
    fs::write(output_path.join("app.js"), generate_js())?;
    fs::write(output_path.join("sitemap.xml"), generate_sitemap(&movies))?;
    fs::write(output_path.join("robots.txt"), generate_robots_txt())?;

    // Write Cloudflare Pages _headers file for cache control
    fs::write(output_path.join("_headers"), generate_headers())?;

    println!(
        "Static site generated: {}/index.html",
        output_path.display()
    );
    println!("  {} movies with OV/OmU screenings", movies.len());
    println!("  {} theaters", theaters.len());

    Ok(())
}

/// Normalizes a theater name for deduplication.
/// Removes common variations, extra whitespace, and standardizes formatting.
fn normalize_theater_name(name: &str) -> String {
    let mut normalized = name.to_lowercase();

    // Remove common prefixes/suffixes that vary between sources
    let removals = [
        "kino in der ",
        "kino im ",
        "kino ",
        " kino",
        " berlin",
        ", berlin",
        " - berlin",
        "berlin - ",
        " - ",
    ];
    for removal in removals {
        normalized = normalized.replace(removal, " ");
    }

    // Standardize German umlauts first
    let umlaut_replacements = [
        ("höfe", "hoefe"),
        ("ü", "ue"),
        ("ö", "oe"),
        ("ä", "ae"),
        ("ß", "ss"),
        ("é", "e"),
    ];
    for (from, to) in umlaut_replacements {
        normalized = normalized.replace(from, to);
    }

    // Standardize known variations
    let replacements = [
        ("cinestar ", "cinestar "),
        ("uci kinowelt ", "uci "),
        ("uci welt ", "uci "),
        ("uci ", "uci "),
        (" | luxe", ""),
        ("| luxe", ""),
        ("|luxe", ""),
        // Kulturbrauerei variations
        ("kulturbrauerei", "kulturbrauerei"),
        // CUBIX variations
        ("cubix am alexanderplatz", "cubix alexanderplatz"),
        ("cubix alexanderplatz", "cubix alexanderplatz"),
        // Passage variations
        ("passage kinos", "passage"),
        ("passage s", "passage"),
        ("passages", "passage"),
        // Wolf variations
        ("wolf kino", "wolf"),
        // Intimes
        ("kino intimes", "intimes"),
        // IL KINO
        ("il kino", "il kino"),
        // East Side Gallery
        ("east side gallery", "east side gallery"),
        ("mercedes platz", "east side gallery"),
        ("mercedes-platz", "east side gallery"),
        // Eastgate
        ("am eastgate", "eastgate"),
        // Gropius
        ("gropius passagen", "gropius"),
        // Remove "am" in middle of names
        (" am ", " "),
    ];
    for (from, to) in replacements {
        normalized = normalized.replace(from, to);
    }

    // Remove extra whitespace and trim
    let normalized = normalized
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();

    // Handle remaining edge cases with specific mappings
    match normalized.as_str() {
        s if s.contains("kulturbrauerei") && !s.contains("cinestar") => {
            "cinestar kulturbrauerei".to_string()
        }
        s if s == "delphi filmpalast zoo" || s == "delphi filmpalast am zoo" => {
            "delphi filmpalast".to_string()
        }
        "il" => "il kino".to_string(),
        "yorck s" | "yorck kinos" | "new yorck" => "yorck".to_string(),
        _ => normalized,
    }
}

/// Determines the chain/group for a theater based on its name.
fn get_theater_chain(name: &str) -> &'static str {
    let name_lower = name.to_lowercase();

    if name_lower.contains("uci") {
        "UCI"
    } else if name_lower.contains("cinestar") {
        "CineStar"
    } else if name_lower.contains("cinemaxx") {
        "CinemaxX"
    } else if name_lower.contains("cineplex") {
        "Cineplex"
    } else if name_lower.contains("cinemotion") {
        "CineMotion"
    } else if is_yorck_theater(&name_lower) {
        "Yorck Kinos"
    } else {
        "Independent Cinemas"
    }
}

/// Checks if a theater is part of the Yorck group.
fn is_yorck_theater(name_lower: &str) -> bool {
    let yorck_theaters = [
        "babylon",
        "capitol",
        "cinema paris",
        "delphi filmpalast",
        "delphi lux",
        "filmtheater am friedrichshain",
        "international",
        "kant",
        "kino",
        "neues off",
        "odeon",
        "passage",
        "rollberg",
        "yorck",
        "new yorck",
    ];

    // Exact matches or contains check
    for yorck in yorck_theaters {
        if name_lower.contains(yorck)
            && !name_lower.contains("uci")
            && !name_lower.contains("cinestar")
        {
            // Special case: "Babylon Kreuzberg" is Yorck, but need to be careful
            if yorck == "babylon" && name_lower.contains("babylon") {
                return true;
            }
            if yorck != "babylon" && name_lower.contains(yorck) {
                return true;
            }
        }
    }

    false
}

fn fetch_theaters(db: &Database) -> Result<Vec<TheaterInfo>> {
    let conn = db.connection();
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, t.url
         FROM theaters t
         ORDER BY t.name",
    )?;

    // Collect all theaters from DB
    struct RawTheater {
        id: i64,
        name: String,
        url: Option<String>,
    }

    let rows = stmt.query_map([], |row| {
        Ok(RawTheater {
            id: row.get(0)?,
            name: row.get(1)?,
            url: row.get(2)?,
        })
    })?;

    let raw_theaters: Vec<RawTheater> = rows.collect::<Result<Vec<_>, _>>()?;

    // Deduplicate by normalized name
    let mut theater_map: HashMap<String, TheaterInfo> = HashMap::new();

    for raw in raw_theaters {
        let normalized = normalize_theater_name(&raw.name);
        let chain = get_theater_chain(&raw.name);

        theater_map
            .entry(normalized.clone())
            .and_modify(|existing| {
                existing.db_ids.push(raw.id);
                // Prefer URLs from the theater's own source (longer URLs tend to be more specific)
                if existing.url.is_none() && raw.url.is_some() {
                    existing.url = raw.url.clone();
                }
                // Prefer display names that are longer/more complete
                if raw.name.len() > existing.display_name.len() {
                    existing.display_name = raw.name.clone();
                }
            })
            .or_insert(TheaterInfo {
                normalized_name: normalized,
                display_name: raw.name,
                url: raw.url,
                chain: chain.to_string(),
                db_ids: vec![raw.id],
            });
    }

    // Convert to vec and sort by chain, then name
    let mut theaters: Vec<TheaterInfo> = theater_map.into_values().collect();
    theaters.sort_by(|a, b| {
        // Sort chains in a specific order
        let chain_order = |c: &str| -> u8 {
            match c {
                "Yorck Kinos" => 0,
                "CineStar" => 1,
                "UCI" => 2,
                "CinemaxX" => 3,
                "Cineplex" => 4,
                "CineMotion" => 5,
                "Independent Cinemas" => 6,
                _ => 7,
            }
        };
        chain_order(&a.chain)
            .cmp(&chain_order(&b.chain))
            .then(a.display_name.cmp(&b.display_name))
    });

    Ok(theaters)
}

fn fetch_ov_movies(db: &Database) -> Result<Vec<MovieData>> {
    let conn = db.connection();
    let now = Utc::now();

    // Get unique movies by TMDB ID (or title if no TMDB ID), consolidating duplicates
    // from different sources/formats into a single entry
    let mut movie_stmt = conn.prepare(
        "SELECT
            MIN(m.id) as id,
            m.title,
            m.english_title,
            m.original_title,
            m.german_title,
            m.original_language,
            m.production_countries,
            m.year,
            MAX(m.runtime_minutes) as runtime_minutes,
            m.genres,
            m.overview,
            m.poster_url,
            m.tmdb_url,
            m.director,
            m.director_id,
            m.writer,
            m.writer_id,
            m.cinematographer,
            m.cinematographer_id,
            COALESCE(m.tmdb_id, -m.id) as group_key
         FROM movies m
         JOIN screenings s ON m.id = s.movie_id
         WHERE (s.is_ov = 1 OR s.is_omu = 1 OR s.is_english_subs = 1)
           AND s.showtime >= ?1
         GROUP BY COALESCE(m.tmdb_id, -m.id)
         ORDER BY COALESCE(m.english_title, m.original_title, m.title)",
    )?;

    let movie_rows = movie_stmt.query_map([now.to_rfc3339()], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<i32>>(7)?,
            row.get::<_, Option<i32>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?,
            row.get::<_, Option<String>>(12)?,
            row.get::<_, Option<String>>(13)?,
            row.get::<_, Option<i32>>(14)?,
            row.get::<_, Option<String>>(15)?,
            row.get::<_, Option<i32>>(16)?,
            row.get::<_, Option<String>>(17)?,
            row.get::<_, Option<i32>>(18)?,
            row.get::<_, i64>(19)?, // group_key for fetching all related movies
        ))
    })?;

    let mut movies = Vec::new();

    for movie_result in movie_rows {
        let (
            id,
            title,
            english_title,
            original_title,
            german_title,
            original_language,
            production_countries,
            year,
            runtime,
            genres,
            overview,
            poster_url,
            tmdb_url,
            director,
            director_id,
            writer,
            writer_id,
            cinematographer,
            cinematographer_id,
            group_key,
        ) = movie_result?;

        // Get screenings for ALL movies with the same TMDB ID (or just this movie if no TMDB ID)
        let screenings: Vec<ScreeningData> = if group_key > 0 {
            // Has TMDB ID - get screenings from all movies with this TMDB ID
            let mut stmt = conn.prepare(
                "SELECT t.name, t.url, s.showtime, s.is_ov, s.is_omu, s.is_english_subs, s.is_3d,
                        s.screening_type, s.booking_url
                 FROM screenings s
                 JOIN theaters t ON s.theater_id = t.id
                 JOIN movies m ON s.movie_id = m.id
                 WHERE m.tmdb_id = ?1
                   AND (s.is_ov = 1 OR s.is_omu = 1 OR s.is_english_subs = 1)
                   AND s.showtime >= ?2
                 ORDER BY s.showtime",
            )?;
            let rows = stmt.query_map([&group_key.to_string(), &now.to_rfc3339()], |row| {
                let showtime_str: String = row.get(2)?;
                let showtime = DateTime::parse_from_rfc3339(&showtime_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or(now);

                let theater_name: String = row.get(0)?;
                Ok(ScreeningData {
                    normalized_theater_name: normalize_theater_name(&theater_name),
                    theater_name,
                    theater_url: row.get(1)?,
                    showtime,
                    is_ov: row.get::<_, i32>(3)? != 0,
                    is_omu: row.get::<_, i32>(4)? != 0,
                    is_english_subs: row.get::<_, i32>(5)? != 0,
                    is_3d: row.get::<_, i32>(6)? != 0,
                    screening_type: row.get(7)?,
                    booking_url: row.get(8)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            // No TMDB ID - just get screenings for this specific movie
            let mut stmt = conn.prepare(
                "SELECT t.name, t.url, s.showtime, s.is_ov, s.is_omu, s.is_english_subs, s.is_3d,
                        s.screening_type, s.booking_url
                 FROM screenings s
                 JOIN theaters t ON s.theater_id = t.id
                 WHERE s.movie_id = ?1
                   AND (s.is_ov = 1 OR s.is_omu = 1 OR s.is_english_subs = 1)
                   AND s.showtime >= ?2
                 ORDER BY s.showtime",
            )?;
            let rows = stmt.query_map([&id.to_string(), &now.to_rfc3339()], |row| {
                let showtime_str: String = row.get(2)?;
                let showtime = DateTime::parse_from_rfc3339(&showtime_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or(now);

                let theater_name: String = row.get(0)?;
                Ok(ScreeningData {
                    normalized_theater_name: normalize_theater_name(&theater_name),
                    theater_name,
                    theater_url: row.get(1)?,
                    showtime,
                    is_ov: row.get::<_, i32>(3)? != 0,
                    is_omu: row.get::<_, i32>(4)? != 0,
                    is_english_subs: row.get::<_, i32>(5)? != 0,
                    is_3d: row.get::<_, i32>(6)? != 0,
                    screening_type: row.get(7)?,
                    booking_url: row.get(8)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };

        movies.push(MovieData {
            id,
            title,
            english_title,
            original_title,
            german_title,
            original_language,
            production_countries,
            year,
            runtime_minutes: runtime,
            genres,
            overview,
            poster_url,
            tmdb_url,
            director,
            director_id,
            writer,
            writer_id,
            cinematographer,
            cinematographer_id,
            screenings,
        });
    }

    Ok(movies)
}

/// Generates JSON-LD structured data for SEO.
fn generate_json_ld(movies: &[MovieData]) -> String {
    let mut items = String::new();
    // Limit to 50 movies to keep JSON-LD size reasonable
    let max_items = movies.len().min(50);

    for (i, movie) in movies.iter().take(max_items).enumerate() {
        let display_title = movie
            .english_title
            .as_ref()
            .or(movie.original_title.as_ref())
            .unwrap_or(&movie.title);

        if i > 0 {
            items.push(',');
        }

        items.push_str(&format!(
            r#"
        {{
          "@type": "ListItem",
          "position": {},
          "item": {{
            "@type": "Movie",
            "name": "{}",
            "url": "https://ovberlin.com/#movie-{}""#,
            i + 1,
            escape_json(display_title),
            movie.id
        ));

        if let Some(ref director) = movie.director {
            items.push_str(&format!(
                r#",
            "director": {{ "@type": "Person", "name": "{}" }}"#,
                escape_json(director)
            ));
        }

        if let Some(year) = movie.year {
            items.push_str(&format!(
                r#",
            "dateCreated": "{}""#,
                year
            ));
        }

        if let Some(ref overview) = movie.overview {
            if !overview.is_empty() {
                let truncated: String = overview.chars().take(200).collect();
                items.push_str(&format!(
                    r#",
            "description": "{}""#,
                    escape_json(&truncated)
                ));
            }
        }

        if let Some(ref poster_url) = movie.poster_url {
            items.push_str(&format!(
                r#",
            "image": "{}""#,
                escape_json(poster_url)
            ));
        }

        items.push_str(
            r#"
          }
        }"#,
        );
    }

    format!(
        r#"<script type="application/ld+json">
    {{
      "@context": "https://schema.org",
      "@graph": [
        {{
          "@type": "WebSite",
          "name": "OV Berlin",
          "url": "https://ovberlin.com/",
          "description": "Find original version (OV) and subtitled (OmU) movie screenings in Berlin"
        }},
        {{
          "@type": "ItemList",
          "name": "OV Movies Showing in Berlin",
          "numberOfItems": {},
          "itemListElement": [{}
          ]
        }}
      ]
    }}
    </script>"#,
        movies.len(),
        items
    )
}

fn generate_html(movies: &[MovieData], theaters: &[TheaterInfo], cache_version: i64) -> String {
    let mut html = String::new();

    let movie_count = movies.len();
    let theater_count = theaters.len();
    let meta_description = format!(
        "Find original version (OV) and subtitled (OmU) movie screenings in Berlin. {} films at {} cinemas, updated twice daily. Free, ad-free, and easy to use.",
        movie_count, theater_count
    );

    // Build JSON-LD structured data
    let json_ld = generate_json_ld(movies);

    // DOCTYPE and head (with cache-busting version on CSS)
    html.push_str(&format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>OV Berlin – Original Version Movies &amp; Showtimes</title>
    <meta name="description" content="{}">
    <link rel="canonical" href="https://ovberlin.com/">
    <meta property="og:type" content="website">
    <meta property="og:url" content="https://ovberlin.com/">
    <meta property="og:title" content="OV Berlin – Original Version Movies &amp; Showtimes">
    <meta property="og:description" content="{}">
    <meta property="og:locale" content="en_US">
    <meta property="og:site_name" content="OV Berlin">
    <meta name="twitter:card" content="summary">
    <meta name="twitter:title" content="OV Berlin – Original Version Movies &amp; Showtimes">
    <meta name="twitter:description" content="{}">
    <link rel="stylesheet" href="style.css?v={}">
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&display=swap" rel="stylesheet">
    {}
</head>
<body>
    <header class="site-header">
        <h1>OV Berlin</h1>
        <p class="tagline">Original version movie screenings in Berlin</p>
    </header>"#,
        escape_html(&meta_description),
        escape_html(&meta_description),
        escape_html(&meta_description),
        cache_version,
        json_ld,
    ));

    html.push_str(
        r#"
    <nav class="theater-filter">
        <details>
            <summary>Filter by Theater</summary>
            <fieldset>
                <legend>Select theaters to show</legend>
                <button type="button" id="select-all">Select All</button>
                <button type="button" id="select-none">Clear All</button>
                <ul class="theater-list">
"#,
    );

    // Group theaters by chain
    let mut theaters_by_chain: HashMap<String, Vec<&TheaterInfo>> = HashMap::new();
    for theater in theaters {
        theaters_by_chain
            .entry(theater.chain.clone())
            .or_default()
            .push(theater);
    }

    // Sort chains in preferred order
    let chain_order = [
        "Yorck Kinos",
        "CineStar",
        "UCI",
        "CinemaxX",
        "Cineplex",
        "CineMotion",
        "Independent Cinemas",
    ];
    for chain in chain_order {
        if let Some(chain_theaters) = theaters_by_chain.get(chain) {
            html.push_str(&format!(
                r#"                    <li class="theater-group">
                        <strong>{}</strong>
                        <ul>
"#,
                escape_html(chain)
            ));

            for theater in chain_theaters {
                // Use normalized name as the value for filtering (matches screening data-theater)
                html.push_str(&format!(
                    r#"                            <li>
                                <label>
                                    <input type="checkbox" name="theater" value="{}" data-name="{}" checked>
                                    {}
                                </label>
                            </li>
"#,
                    escape_html(&theater.normalized_name),
                    escape_html(&theater.display_name),
                    escape_html(&theater.display_name)
                ));
            }

            html.push_str(
                r#"                        </ul>
                    </li>
"#,
            );
        }
    }

    html.push_str(
        r#"                </ul>
            </fieldset>
        </details>
        <div class="filter-controls">
            <div class="search-box">
                <input type="search" id="search" placeholder="Search movies, directors, writers..." autocomplete="off">
            </div>
            <fieldset class="date-filter">
                <legend>Filter by date</legend>
                <label>
                    <input type="checkbox" id="filter-today" name="date-filter" value="today">
                    Today
                </label>
                <label>
                    <input type="checkbox" id="filter-tomorrow" name="date-filter" value="tomorrow">
                    Tomorrow
                </label>
            </fieldset>
            <label class="show-all-toggle">
                <input type="checkbox" id="show-all-ov">
                Show all OV films (including non-English)
            </label>
        </div>
    </nav>

    <main class="movie-grid">
"#,
    );

    // Generate movie cards
    for movie in movies {
        // Use English title for display, fall back to original title, then scraped title
        let display_title = movie
            .english_title
            .as_ref()
            .or(movie.original_title.as_ref())
            .unwrap_or(&movie.title);
        let year_str = movie.year.map(|y| format!(" ({})", y)).unwrap_or_default();
        let runtime_str = movie
            .runtime_minutes
            .map(|m| format!("{} min", m))
            .unwrap_or_default();

        let poster_html = if let Some(ref url) = movie.poster_url {
            format!(
                r#"<img src="{}" alt="{}" loading="lazy">"#,
                escape_html(url),
                escape_html(display_title)
            )
        } else {
            // Check for special event types when no TMDB poster
            let title_lower = movie.title.to_lowercase();
            if title_lower.contains("sneak") || title_lower == "classic sneak" {
                r#"<span class="no-poster sneak-preview"><svg viewBox="0 0 100 100" fill="currentColor"><circle cx="50" cy="35" r="20" fill="none" stroke="currentColor" stroke-width="4"/><path d="M30 35 Q30 55 50 55 Q70 55 70 35" fill="none" stroke="currentColor" stroke-width="4"/><circle cx="50" cy="35" r="8"/><text x="50" y="80" text-anchor="middle" font-size="12" font-weight="bold">SNEAK</text><text x="50" y="93" text-anchor="middle" font-size="10">PREVIEW</text></svg></span>"#.to_string()
            } else if title_lower.contains("festival") || title_lower.contains("filmfest") {
                r#"<span class="no-poster film-festival"><svg viewBox="0 0 100 100" fill="currentColor"><rect x="35" y="15" width="30" height="45" rx="3" fill="none" stroke="currentColor" stroke-width="3"/><circle cx="42" cy="25" r="4"/><circle cx="42" cy="35" r="4"/><circle cx="42" cy="45" r="4"/><circle cx="58" cy="25" r="4"/><circle cx="58" cy="35" r="4"/><circle cx="58" cy="45" r="4"/><path d="M25 60 L50 75 L75 60" fill="none" stroke="currentColor" stroke-width="3"/><text x="50" y="90" text-anchor="middle" font-size="10" font-weight="bold">FILM FESTIVAL</text></svg></span>"#.to_string()
            } else {
                format!(
                    r#"<span class="no-poster">{}</span>"#,
                    escape_html(&display_title.chars().next().unwrap_or('?').to_string())
                )
            }
        };

        // Collect normalized theater names for this movie (for filtering)
        let theater_names: Vec<String> = movie
            .screenings
            .iter()
            .map(|s| s.normalized_theater_name.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let theater_names_str = theater_names.join(",");

        // Check if movie is English (original language is "en" or has English subtitles)
        let is_english_language = movie
            .original_language
            .as_ref()
            .map(|l| l == "en")
            .unwrap_or(false);
        let has_english_subs = movie.screenings.iter().any(|s| s.is_english_subs);
        let is_english = is_english_language || has_english_subs;

        // Build search text for filtering (lowercase for case-insensitive search)
        let search_text = format!(
            "{} {} {} {}",
            display_title,
            movie.director.as_deref().unwrap_or(""),
            movie.writer.as_deref().unwrap_or(""),
            movie.cinematographer.as_deref().unwrap_or("")
        )
        .to_lowercase();

        // Collect unique screening dates for date filtering (YYYY-MM-DD format)
        let screening_dates: Vec<String> = movie
            .screenings
            .iter()
            .map(|s| {
                s.showtime
                    .with_timezone(&Berlin)
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let screening_dates_str = screening_dates.join(",");

        // Determine if we should show the original title (when different from display)
        let original_title_display = movie.original_title.as_ref().and_then(|orig| {
            if orig != display_title {
                Some(orig.as_str())
            } else {
                None
            }
        });

        html.push_str(&format!(
            r#"        <article class="movie-card" id="movie-{}" data-movie-id="{}" data-theaters="{}" data-english="{}" data-search="{}" data-dates="{}">
            <figure class="movie-poster">
                {}
            </figure>
            <header class="movie-header">
                <h2 class="movie-title-row">{}{}{}</h2>
"#,
            movie.id,
            movie.id,
            escape_html(&theater_names_str),
            is_english,
            escape_html(&search_text),
            screening_dates_str,
            poster_html,
            escape_html(display_title),
            year_str,
            // Add TMDB link icon inline with title (only shown when expanded via CSS)
            if let Some(ref tmdb_url) = movie.tmdb_url {
                format!(
                    "<a href=\"{}\" target=\"_blank\" rel=\"noopener\" class=\"tmdb-link\" title=\"View on TMDB\">\
                    <svg class=\"external-icon\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\">\
                    <path d=\"M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6\"/>\
                    <polyline points=\"15 3 21 3 21 9\"/>\
                    <line x1=\"10\" y1=\"14\" x2=\"21\" y2=\"3\"/>\
                    </svg></a>",
                    escape_html(tmdb_url)
                )
            } else {
                String::new()
            }
        ));

        // Add original title as subtitle (only shown when expanded, hidden by default via CSS)
        if let Some(orig_title) = original_title_display {
            html.push_str(&format!(
                r#"                <p class="original-title">Original: {}</p>
"#,
                escape_html(orig_title)
            ));
        }

        // Add German title (only shown when expanded)
        let german_title_display = movie.german_title.as_ref().and_then(|german| {
            // Only show if different from both display title and original title
            if german != display_title && movie.original_title.as_ref() != Some(german) {
                Some(german.as_str())
            } else {
                None
            }
        });
        if let Some(german) = german_title_display {
            html.push_str(&format!(
                r#"                <p class="german-title">German: {}</p>
"#,
                escape_html(german)
            ));
        }

        // Add language and country info (only shown when expanded)
        let has_lang_country =
            movie.original_language.is_some() || movie.production_countries.is_some();
        if has_lang_country {
            html.push_str(r#"                <p class="language-country">"#);
            if let Some(ref lang) = movie.original_language {
                // Convert language code to full name for common languages
                let lang_name = match lang.as_str() {
                    "en" => "English",
                    "de" => "German",
                    "fr" => "French",
                    "es" => "Spanish",
                    "it" => "Italian",
                    "ja" => "Japanese",
                    "ko" => "Korean",
                    "zh" => "Chinese",
                    "ru" => "Russian",
                    "pt" => "Portuguese",
                    "hi" => "Hindi",
                    "ar" => "Arabic",
                    "tr" => "Turkish",
                    "pl" => "Polish",
                    "nl" => "Dutch",
                    "sv" => "Swedish",
                    "da" => "Danish",
                    "no" => "Norwegian",
                    "fi" => "Finnish",
                    "cs" => "Czech",
                    "hu" => "Hungarian",
                    "el" => "Greek",
                    "he" => "Hebrew",
                    "th" => "Thai",
                    "vi" => "Vietnamese",
                    "id" => "Indonesian",
                    "uk" => "Ukrainian",
                    "ro" => "Romanian",
                    "fa" => "Persian",
                    "bn" => "Bengali",
                    "ta" => "Tamil",
                    "te" => "Telugu",
                    "mr" => "Marathi",
                    "cn" => "Cantonese",
                    "tl" => "Tagalog",
                    _ => lang.as_str(),
                };
                html.push_str(&format!(
                    r#"<span class="original-lang">{}</span>"#,
                    escape_html(lang_name)
                ));
            }
            if let Some(ref countries) = movie.production_countries {
                if movie.original_language.is_some() {
                    html.push_str(r#"<span class="separator"> · </span>"#);
                }
                html.push_str(&format!(
                    r#"<span class="countries">{}</span>"#,
                    escape_html(countries)
                ));
            }
            html.push_str("</p>\n");
        }

        if !runtime_str.is_empty() || movie.genres.is_some() {
            html.push_str(r#"                <p class="movie-meta">"#);
            if let Some(ref genres) = movie.genres {
                html.push_str(&format!(
                    r#"<span class="genres">{}</span>"#,
                    escape_html(genres)
                ));
            }
            if !runtime_str.is_empty() {
                html.push_str(&format!(
                    r#"<span class="runtime">{}</span>"#,
                    escape_html(&runtime_str)
                ));
            }
            html.push_str("</p>\n");
        }

        html.push_str("            </header>\n");

        // Crew info with TMDB profile links
        let has_crew =
            movie.director.is_some() || movie.writer.is_some() || movie.cinematographer.is_some();
        if has_crew {
            html.push_str(r#"            <dl class="movie-crew">"#);
            if let Some(ref director) = movie.director {
                let director_html = if let Some(id) = movie.director_id {
                    format!(
                        r#"<a href="https://www.themoviedb.org/person/{}" target="_blank" rel="noopener">{}</a>"#,
                        id,
                        escape_html(director)
                    )
                } else {
                    escape_html(director)
                };
                html.push_str(&format!(r#"<dt>Director</dt><dd>{}</dd>"#, director_html));
            }
            if let Some(ref writer) = movie.writer {
                let writer_html = if let Some(id) = movie.writer_id {
                    format!(
                        r#"<a href="https://www.themoviedb.org/person/{}" target="_blank" rel="noopener">{}</a>"#,
                        id,
                        escape_html(writer)
                    )
                } else {
                    escape_html(writer)
                };
                html.push_str(&format!(r#"<dt>Writer</dt><dd>{}</dd>"#, writer_html));
            }
            if let Some(ref cinematographer) = movie.cinematographer {
                let cinematographer_html = if let Some(id) = movie.cinematographer_id {
                    format!(
                        r#"<a href="https://www.themoviedb.org/person/{}" target="_blank" rel="noopener">{}</a>"#,
                        id,
                        escape_html(cinematographer)
                    )
                } else {
                    escape_html(cinematographer)
                };
                html.push_str(&format!(
                    r#"<dt>Cinematography</dt><dd>{}</dd>"#,
                    cinematographer_html
                ));
            }
            html.push_str("</dl>\n");
        }

        // Overview
        if let Some(ref overview) = movie.overview {
            if !overview.is_empty() {
                let truncated = if overview.chars().count() > 200 {
                    format!("{}...", overview.chars().take(200).collect::<String>())
                } else {
                    overview.clone()
                };
                html.push_str(&format!(
                    r#"            <p class="movie-overview">{}</p>
"#,
                    escape_html(&truncated)
                ));
            }
        }

        // Screenings (hidden by default, shown on click)
        html.push_str(
            r#"            <section class="movie-screenings" hidden>
                <h3>Showtimes</h3>
"#,
        );

        // Group screenings by date
        let mut screenings_by_date: HashMap<String, Vec<&ScreeningData>> = HashMap::new();
        for screening in &movie.screenings {
            let local_time = screening.showtime.with_timezone(&Berlin);
            let date_key = local_time.format("%Y-%m-%d").to_string();
            screenings_by_date
                .entry(date_key)
                .or_default()
                .push(screening);
        }

        let mut dates: Vec<_> = screenings_by_date.keys().collect();
        dates.sort();

        for date in dates {
            let screenings = &screenings_by_date[date];
            let first_screening = screenings[0];
            let local_time = first_screening.showtime.with_timezone(&Berlin);
            let date_display = local_time.format("%A, %B %d").to_string();

            html.push_str(&format!(
                r#"                <div class="screening-day" data-date="{}">
                    <h4>{}</h4>
                    <ul class="screening-times">
"#,
                date, // YYYY-MM-DD format for JS filtering
                date_display
            ));

            for screening in screenings {
                let local_time = screening.showtime.with_timezone(&Berlin);
                let time_str = local_time.format("%H:%M").to_string();

                let mut tags = Vec::new();
                if screening.is_ov {
                    tags.push("OV");
                }
                if screening.is_omu {
                    tags.push("OmU");
                }
                if screening.is_english_subs {
                    tags.push("OmeU");
                }
                if screening.is_3d {
                    tags.push("3D");
                }
                if let Some(ref t) = screening.screening_type {
                    tags.push(t);
                }
                let tags_str = tags.join(" ");

                let booking_html = if let Some(ref url) = screening.booking_url {
                    format!(
                        r#" <a href="{}" target="_blank" rel="noopener">Book</a>"#,
                        escape_html(url)
                    )
                } else {
                    String::new()
                };

                let theater_html = if let Some(ref url) = screening.theater_url {
                    format!(
                        r#"<a href="{}" target="_blank" rel="noopener">{}</a>"#,
                        escape_html(url),
                        escape_html(&screening.theater_name)
                    )
                } else {
                    escape_html(&screening.theater_name)
                };

                html.push_str(&format!(
                    r#"                        <li class="screening" data-theater="{}">
                            <time datetime="{}">{}</time>
                            <span class="theater-name">{}</span>
                            <span class="screening-tags">{}</span>{}
                        </li>
"#,
                    escape_html(&screening.normalized_theater_name),
                    screening.showtime.to_rfc3339(),
                    time_str,
                    theater_html,
                    tags_str,
                    booking_html
                ));
            }

            html.push_str(
                r#"                    </ul>
                </div>
"#,
            );
        }

        html.push_str(
            r#"            </section>
        </article>
"#,
        );
    }

    // Get current time for "last updated"
    let now = Utc::now().with_timezone(&Berlin);
    let updated_str = now.format("%B %d, %Y at %H:%M").to_string();

    html.push_str(&format!(
        r#"    </main>

    <footer class="site-footer">
        <p>Developed for Film Lovers by the filmmakers at: <a href="https://secedastudios.com" target="_blank" rel="noopener">Seceda Studios</a><br/>
            If you are also a filmmaker/actor/creator/crew please join us on our 100% Free/Ad Free Directory: <a href="https://slatehub.com" target="_blank" rel="noopener">SlateHub</a></p>
        <p class="last-updated">Last updated: {}</p>
    </footer>

    <script src="app.js?v={}"></script>
    <script src='https://storage.ko-fi.com/cdn/scripts/overlay-widget.js'></script>
    <script>
      kofiWidgetOverlay.draw('chrisabruce', {{
        'type': 'floating-chat',
        'floating-chat.donateButton.text': 'Support Me',
        'floating-chat.donateButton.background-color': '#ff38b8',
        'floating-chat.donateButton.text-color': '#fff'
      }});
    </script>
</body>
</html>
"#,
        updated_str, cache_version
    ));

    html
}

fn generate_css() -> &'static str {
    r#"/* Film Finder - Static Site Styles */

:root {
    --bg-primary: #1a1a2e;
    --bg-secondary: #16213e;
    --bg-card: #1f2937;
    --bg-hover: #374151;
    --text-primary: #f3f4f6;
    --text-secondary: #9ca3af;
    --text-muted: #6b7280;
    --accent: #3b82f6;
    --accent-hover: #60a5fa;
    --border: #374151;
    --tag-bg: #4b5563;
    --tag-ov: #059669;
    --tag-3d: #7c3aed;
}

* {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
}

html {
    font-size: 16px;
    scroll-behavior: smooth;
}

body {
    font-family: 'Inter', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
    background-color: var(--bg-primary);
    color: var(--text-primary);
    line-height: 1.6;
    min-height: 100vh;
}

/* Header */
.site-header {
    text-align: center;
    padding: 2rem 1rem;
    background: linear-gradient(180deg, var(--bg-secondary) 0%, var(--bg-primary) 100%);
    border-bottom: 1px solid var(--border);
}

.site-header h1 {
    font-size: 2rem;
    font-weight: 600;
    letter-spacing: -0.02em;
    margin-bottom: 0.25rem;
}

.tagline {
    color: var(--text-secondary);
    font-size: 1rem;
}

/* Theater Filter */
.theater-filter {
    max-width: 1400px;
    margin: 0 auto;
    padding: 1rem;
}

.theater-filter details {
    background: var(--bg-card);
    border-radius: 0.5rem;
    border: 1px solid var(--border);
}

.theater-filter summary {
    padding: 1rem;
    cursor: pointer;
    font-weight: 500;
    user-select: none;
}

.theater-filter summary:hover {
    background: var(--bg-hover);
    border-radius: 0.5rem;
}

.theater-filter fieldset {
    border: none;
    padding: 0 1rem 1rem;
}

.theater-filter legend {
    color: var(--text-secondary);
    font-size: 0.875rem;
    margin-bottom: 0.5rem;
}

.theater-filter button {
    background: var(--bg-hover);
    color: var(--text-primary);
    border: 1px solid var(--border);
    padding: 0.5rem 1rem;
    border-radius: 0.25rem;
    cursor: pointer;
    font-size: 0.875rem;
    margin-right: 0.5rem;
    margin-bottom: 0.5rem;
    transition: background 0.15s;
}

.theater-filter button:hover {
    background: var(--accent);
}

/* Filter controls */
.filter-controls {
    margin-top: 1rem;
    display: flex;
    flex-wrap: wrap;
    gap: 1rem;
    align-items: center;
}

.search-box {
    flex: 1;
    min-width: 200px;
}

.search-box input {
    width: 100%;
    padding: 0.75rem 1rem;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 0.5rem;
    color: var(--text-primary);
    font-size: 0.875rem;
    font-family: inherit;
}

.search-box input:focus {
    outline: none;
    border-color: var(--accent);
}

.search-box input::placeholder {
    color: var(--text-muted);
}

.date-filter {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.5rem 1rem;
    background: var(--bg-card);
    border-radius: 0.5rem;
    border: 1px solid var(--border);
}

.date-filter legend {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
}

.date-filter label {
    display: flex;
    align-items: center;
    gap: 0.375rem;
    cursor: pointer;
    font-size: 0.875rem;
    padding: 0.25rem 0.5rem;
    border-radius: 0.25rem;
    transition: background 0.15s;
}

.date-filter label:hover {
    background: var(--bg-hover);
}

.date-filter input[type="checkbox"] {
    accent-color: var(--accent);
    width: 1rem;
    height: 1rem;
}

.show-all-toggle {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    cursor: pointer;
    font-size: 0.875rem;
    padding: 0.75rem 1rem;
    background: var(--bg-card);
    border-radius: 0.5rem;
    border: 1px solid var(--border);
    white-space: nowrap;
}

.show-all-toggle input[type="checkbox"] {
    accent-color: var(--accent);
    width: 1rem;
    height: 1rem;
}

.theater-list {
    list-style: none;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: 1rem;
    margin-top: 1rem;
}

.theater-group {
    background: var(--bg-secondary);
    padding: 0.75rem;
    border-radius: 0.375rem;
}

.theater-group strong {
    display: block;
    margin-bottom: 0.5rem;
    color: var(--accent);
    font-size: 0.875rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
}

.theater-group ul {
    list-style: none;
}

.theater-group label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.25rem 0;
    font-size: 0.875rem;
    cursor: pointer;
}

.theater-group input[type="checkbox"] {
    accent-color: var(--accent);
}

/* Movie Grid */
.movie-grid {
    max-width: 1400px;
    margin: 0 auto;
    padding: 1rem;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 1.5rem;
}

/* Movie Card */
.movie-card {
    background: var(--bg-card);
    border-radius: 0.5rem;
    overflow: hidden;
    border: 1px solid var(--border);
    transition: transform 0.2s, box-shadow 0.2s;
    cursor: pointer;
}

.movie-card:hover {
    transform: translateY(-2px);
    box-shadow: 0 8px 25px rgba(0, 0, 0, 0.3);
}

.movie-card.expanded {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: 300px 1fr;
    grid-template-rows: auto 1fr;
    cursor: default;
}

.movie-card.hidden {
    display: none;
}

.movie-poster {
    aspect-ratio: 2/3;
    background: var(--bg-secondary);
    overflow: hidden;
}

.movie-card.expanded .movie-poster {
    grid-row: 1 / 3;
}

.movie-poster img {
    width: 100%;
    height: 100%;
    object-fit: cover;
}

.no-poster {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    font-size: 4rem;
    font-weight: 600;
    color: var(--text-muted);
    background: linear-gradient(135deg, var(--bg-secondary), var(--bg-hover));
}

.no-poster.sneak-preview {
    background: linear-gradient(135deg, #1a1a2e, #2d1b4e);
    color: #a78bfa;
}

.no-poster.sneak-preview svg {
    width: 60%;
    height: auto;
}

.no-poster.film-festival {
    background: linear-gradient(135deg, #1a2e1a, #2e4a1a);
    color: #86efac;
}

.no-poster.film-festival svg {
    width: 60%;
    height: auto;
}

.movie-header {
    padding: 1rem;
}

.movie-header h2 {
    font-size: 1.125rem;
    font-weight: 600;
    margin-bottom: 0.25rem;
    line-height: 1.3;
}

.movie-title-row {
    display: flex;
    align-items: baseline;
    gap: 0.5rem;
    flex-wrap: wrap;
}

.original-title,
.german-title {
    display: none;
    font-size: 0.875rem;
    color: var(--text-muted);
    font-style: italic;
    margin-bottom: 0.125rem;
}

.movie-card.expanded .original-title,
.movie-card.expanded .german-title {
    display: block;
}

.language-country {
    display: none;
    font-size: 0.8125rem;
    color: var(--text-secondary);
    margin-top: 0.5rem;
    margin-bottom: 0.25rem;
}

.movie-card.expanded .language-country {
    display: block;
}

.language-country .original-lang {
    font-weight: 500;
}

.language-country .separator {
    color: var(--text-muted);
}

.tmdb-link {
    display: none;
    align-items: center;
    color: var(--text-muted);
    transition: color 0.15s;
}

.movie-card.expanded .tmdb-link {
    display: inline-flex;
}

.tmdb-link:hover {
    color: var(--accent);
}

.external-icon {
    width: 0.875em;
    height: 0.875em;
    vertical-align: baseline;
}

.movie-meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem;
    font-size: 0.8125rem;
    color: var(--text-secondary);
}

.movie-meta .genres {
    color: var(--accent);
}

.movie-meta .runtime::before {
    content: "·";
    margin-right: 0.5rem;
}

.movie-crew {
    padding: 0 1rem 0.5rem;
    font-size: 0.8125rem;
}

.movie-crew dt {
    color: var(--text-muted);
    font-weight: 500;
    margin-top: 0.25rem;
}

.movie-crew dt:first-child {
    margin-top: 0;
}

.movie-crew dd {
    color: var(--text-secondary);
    margin: 0;
}

.movie-crew dd a {
    color: var(--text-secondary);
    text-decoration: none;
    transition: color 0.15s;
}

.movie-crew dd a:hover {
    color: var(--accent);
    text-decoration: underline;
}

.movie-overview {
    padding: 0 1rem 1rem;
    font-size: 0.875rem;
    color: var(--text-secondary);
    line-height: 1.5;
}

/* Screenings */
.movie-screenings {
    padding: 1rem;
    border-top: 1px solid var(--border);
    max-height: 60vh;
    overflow-y: auto;
}

.movie-screenings h3 {
    font-size: 1rem;
    font-weight: 600;
    margin-bottom: 1rem;
    color: var(--accent);
}

.screening-day {
    margin-bottom: 1.5rem;
}

.screening-day:last-child {
    margin-bottom: 0;
}

.screening-day.hidden {
    display: none;
}

.screening-day h4 {
    font-size: 0.875rem;
    font-weight: 500;
    color: var(--text-secondary);
    margin-bottom: 0.5rem;
    padding-bottom: 0.25rem;
    border-bottom: 1px solid var(--border);
}

.screening-times {
    list-style: none;
}

.screening {
    display: grid;
    grid-template-columns: auto 1fr auto auto;
    gap: 0.75rem;
    align-items: center;
    padding: 0.5rem 0;
    font-size: 0.875rem;
    border-bottom: 1px solid var(--bg-hover);
}

.screening:last-child {
    border-bottom: none;
}

.screening.hidden {
    display: none;
}

.screening time {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text-primary);
}

.theater-name {
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
}

.theater-name a {
    color: var(--text-secondary);
    text-decoration: none;
    transition: color 0.15s;
}

.theater-name a:hover {
    color: var(--accent);
    text-decoration: underline;
}

.screening-tags {
    display: flex;
    gap: 0.25rem;
    font-size: 0.75rem;
    font-weight: 500;
    color: var(--text-primary);
    background: var(--tag-ov);
    padding: 0.125rem 0.375rem;
    border-radius: 0.25rem;
}

.screening a {
    color: var(--accent);
    text-decoration: none;
    font-weight: 500;
    font-size: 0.8125rem;
}

.screening a:hover {
    color: var(--accent-hover);
    text-decoration: underline;
}

/* Footer */
.site-footer {
    text-align: center;
    padding: 2rem 1rem;
    color: var(--text-muted);
    font-size: 0.8125rem;
    border-top: 1px solid var(--border);
    margin-top: 2rem;
}

.last-updated {
    margin-top: 0.5rem;
    font-size: 0.75rem;
    opacity: 0.7;
}

.site-footer a,
.site-footer a:visited {
    color: #6b7280;
}

.site-footer a:hover {
    color: #9ca3af;
}

/* Mobile adjustments */
@media (max-width: 768px) {
    .site-header h1 {
        font-size: 1.5rem;
    }

    .movie-grid {
        grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
        gap: 1rem;
    }

    .movie-card.expanded {
        grid-template-columns: 1fr;
    }

    .movie-card.expanded .movie-poster {
        grid-row: auto;
        max-height: 300px;
    }

    .movie-header h2 {
        font-size: 1rem;
    }

    .screening {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: 0.25rem 0.5rem;
        padding: 0.5rem 0;
        font-size: 0.875rem;
        border-bottom: 1px solid var(--bg-hover);
    }

    .screening time {
        min-width: 3rem;
    }

    .theater-name {
        flex: 1;
        min-width: 0;
    }

    .screening-tags {
        width: auto;
    }

    .screening a {
        margin-left: auto;
    }

    .theater-list {
        grid-template-columns: 1fr;
    }
}

/* Scrollbar styling */
::-webkit-scrollbar {
    width: 8px;
    height: 8px;
}

::-webkit-scrollbar-track {
    background: var(--bg-primary);
}

::-webkit-scrollbar-thumb {
    background: var(--border);
    border-radius: 4px;
}

::-webkit-scrollbar-thumb:hover {
    background: var(--text-muted);
}
"#
}

fn generate_js() -> &'static str {
    r#"// Film Finder - Interactive functionality

document.addEventListener('DOMContentLoaded', () => {
    const movieCards = document.querySelectorAll('.movie-card');
    const theaterCheckboxes = document.querySelectorAll('input[name="theater"]');
    const dateFilterCheckboxes = document.querySelectorAll('input[name="date-filter"]');
    const filterTodayCheckbox = document.getElementById('filter-today');
    const filterTomorrowCheckbox = document.getElementById('filter-tomorrow');
    const selectAllBtn = document.getElementById('select-all');
    const selectNoneBtn = document.getElementById('select-none');
    const showAllOvCheckbox = document.getElementById('show-all-ov');
    const searchInput = document.getElementById('search');

    const STORAGE_KEY = 'filmFinderPrefs';

    // Get date strings in YYYY-MM-DD format for Berlin timezone
    function getDateStrings() {
        const now = new Date();
        // Format for Berlin timezone
        const formatter = new Intl.DateTimeFormat('en-CA', {
            timeZone: 'Europe/Berlin',
            year: 'numeric',
            month: '2-digit',
            day: '2-digit'
        });
        const today = formatter.format(now);
        const tomorrow = formatter.format(new Date(now.getTime() + 24 * 60 * 60 * 1000));
        return { today, tomorrow };
    }

    // Check if localStorage is available (Safari private mode throws on access)
    function storageAvailable() {
        try {
            const test = '__storage_test__';
            localStorage.setItem(test, test);
            localStorage.removeItem(test);
            return true;
        } catch (e) {
            return false;
        }
    }

    const canUseStorage = storageAvailable();

    // Load saved preferences from localStorage
    function loadPreferences() {
        if (!canUseStorage) return null;
        try {
            const saved = localStorage.getItem(STORAGE_KEY);
            if (!saved) return null;
            return JSON.parse(saved);
        } catch (e) {
            return null;
        }
    }

    // Save preferences to localStorage (by theater name for stability across DB resets)
    function savePreferences() {
        if (!canUseStorage) return;
        const prefs = {
            selectedTheaters: Array.from(theaterCheckboxes)
                .filter(function(cb) { return cb.checked; })
                .map(function(cb) { return cb.dataset.name || cb.value; }),
            showAllOv: showAllOvCheckbox ? showAllOvCheckbox.checked : false
        };
        try {
            localStorage.setItem(STORAGE_KEY, JSON.stringify(prefs));
        } catch (e) {
            // Ignore storage errors
        }
    }

    // Apply saved preferences
    function applyPreferences() {
        const prefs = loadPreferences();
        if (!prefs) return;

        // Apply theater selections (match by name for stability)
        if (prefs.selectedTheaters && Array.isArray(prefs.selectedTheaters)) {
            const selected = new Set(prefs.selectedTheaters);
            theaterCheckboxes.forEach(function(cb) {
                var name = cb.dataset.name || cb.value;
                cb.checked = selected.has(name);
            });
        }

        // Apply show all OV setting
        if (showAllOvCheckbox && typeof prefs.showAllOv === 'boolean') {
            showAllOvCheckbox.checked = prefs.showAllOv;
        }
    }

    // Movie card expansion
    movieCards.forEach(card => {
        card.addEventListener('click', (e) => {
            // Don't collapse if clicking a link or input
            if (e.target.tagName === 'A' || e.target.tagName === 'INPUT') return;

            const isExpanded = card.classList.contains('expanded');

            // Collapse all other cards
            movieCards.forEach(c => {
                c.classList.remove('expanded');
                const screenings = c.querySelector('.movie-screenings');
                if (screenings) screenings.hidden = true;
            });

            // Toggle this card
            if (!isExpanded) {
                card.classList.add('expanded');
                const screenings = card.querySelector('.movie-screenings');
                if (screenings) {
                    screenings.hidden = false;
                    // Scroll the card into view
                    setTimeout(() => {
                        card.scrollIntoView({ behavior: 'smooth', block: 'start' });
                    }, 100);
                }
            }

            updateScreeningsVisibility();
        });
    });

    // Filter functionality (theaters + English/all OV + search + date)
    function updateMoviesVisibility() {
        const selectedTheaters = new Set(
            Array.from(theaterCheckboxes)
                .filter(cb => cb.checked)
                .map(cb => cb.value)
        );
        const showAllOv = showAllOvCheckbox?.checked || false;
        const searchQuery = (searchInput?.value || '').toLowerCase().trim();

        // Date filtering
        const filterToday = filterTodayCheckbox?.checked || false;
        const filterTomorrow = filterTomorrowCheckbox?.checked || false;
        const dateFilterActive = filterToday || filterTomorrow;
        const { today, tomorrow } = getDateStrings();
        const allowedDates = new Set();
        if (filterToday) allowedDates.add(today);
        if (filterTomorrow) allowedDates.add(tomorrow);

        movieCards.forEach(card => {
            const movieTheaters = card.dataset.theaters.split(',');
            const hasSelectedTheater = movieTheaters.some(t => selectedTheaters.has(t));
            const isEnglish = card.dataset.english === 'true';
            const searchText = card.dataset.search || '';

            // Check search match
            const matchesSearch = searchQuery === '' || searchText.includes(searchQuery);

            // Default is English only; if "Show all OV" is checked, show all
            const matchesLanguage = showAllOv || isEnglish;

            // Check date filter - if active, movie must have screenings on selected dates
            const movieDates = (card.dataset.dates || '').split(',').filter(d => d);
            const matchesDate = !dateFilterActive || movieDates.some(d => allowedDates.has(d));

            // Hide if no selected theater OR doesn't match language filter OR doesn't match search OR doesn't match date
            const shouldHide = !hasSelectedTheater || !matchesLanguage || !matchesSearch || !matchesDate;
            card.classList.toggle('hidden', shouldHide);
        });

        updateScreeningsVisibility();
    }

    function updateScreeningsVisibility() {
        const selectedTheaters = new Set(
            Array.from(theaterCheckboxes)
                .filter(cb => cb.checked)
                .map(cb => cb.value)
        );

        // Date filtering for screening days
        const filterToday = filterTodayCheckbox?.checked || false;
        const filterTomorrow = filterTomorrowCheckbox?.checked || false;
        const dateFilterActive = filterToday || filterTomorrow;
        const { today, tomorrow } = getDateStrings();
        const allowedDates = new Set();
        if (filterToday) allowedDates.add(today);
        if (filterTomorrow) allowedDates.add(tomorrow);

        // Filter individual screenings by theater
        document.querySelectorAll('.screening').forEach(screening => {
            const theaterId = screening.dataset.theater;
            screening.classList.toggle('hidden', !selectedTheaters.has(theaterId));
        });

        // Filter screening days by date
        document.querySelectorAll('.screening-day').forEach(day => {
            const dayDate = day.dataset.date;
            const matchesDate = !dateFilterActive || allowedDates.has(dayDate);
            day.classList.toggle('hidden', !matchesDate);
        });
    }

    theaterCheckboxes.forEach(cb => {
        cb.addEventListener('change', () => {
            updateMoviesVisibility();
            savePreferences();
        });
    });

    showAllOvCheckbox?.addEventListener('change', () => {
        updateMoviesVisibility();
        savePreferences();
    });

    // Date filter checkboxes
    dateFilterCheckboxes.forEach(cb => {
        cb.addEventListener('change', updateMoviesVisibility);
    });

    searchInput?.addEventListener('input', updateMoviesVisibility);

    selectAllBtn?.addEventListener('click', () => {
        theaterCheckboxes.forEach(cb => cb.checked = true);
        updateMoviesVisibility();
        savePreferences();
    });

    selectNoneBtn?.addEventListener('click', () => {
        theaterCheckboxes.forEach(cb => cb.checked = false);
        updateMoviesVisibility();
        savePreferences();
    });

    // Keyboard navigation
    document.addEventListener('keydown', (e) => {
        if (e.key === 'Escape') {
            // Clear search if focused
            if (document.activeElement === searchInput) {
                searchInput.value = '';
                updateMoviesVisibility();
                return;
            }
            // Otherwise collapse expanded cards
            movieCards.forEach(card => {
                card.classList.remove('expanded');
                const screenings = card.querySelector('.movie-screenings');
                if (screenings) screenings.hidden = true;
            });
        }
    });

    // Apply saved preferences and run initial filter
    applyPreferences();
    updateMoviesVisibility();
});
"#
}

fn generate_headers() -> &'static str {
    r#"# Cloudflare Pages cache headers

# HTML should be revalidated frequently
/index.html
  Cache-Control: public, max-age=0, must-revalidate

# CSS and JS use cache-busting query strings, so can be cached longer
/*.css
  Cache-Control: public, max-age=31536000, immutable

/*.js
  Cache-Control: public, max-age=31536000, immutable

# Sitemap updates with each deploy
/sitemap.xml
  Cache-Control: public, max-age=3600, must-revalidate
  Content-Type: application/xml

# Robots is mostly static
/robots.txt
  Cache-Control: public, max-age=86400, must-revalidate
"#
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

fn generate_sitemap(movies: &[MovieData]) -> String {
    let now = Utc::now().format("%Y-%m-%d").to_string();
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
"#,
    );
    xml.push_str(&format!(
        r#"  <url>
    <loc>https://ovberlin.com/</loc>
    <lastmod>{}</lastmod>
    <changefreq>daily</changefreq>
    <priority>1.0</priority>
  </url>
"#,
        now
    ));
    // Add individual movie fragment URLs so search engines know about anchored content
    for movie in movies {
        xml.push_str(&format!(
            r#"  <url>
    <loc>https://ovberlin.com/#movie-{}</loc>
    <lastmod>{}</lastmod>
    <changefreq>daily</changefreq>
    <priority>0.8</priority>
  </url>
"#,
            movie.id, now
        ));
    }
    xml.push_str("</urlset>\n");
    xml
}

fn generate_robots_txt() -> &'static str {
    "User-agent: *\nAllow: /\n\nSitemap: https://ovberlin.com/sitemap.xml\n"
}
