//! Static website generator for browsing OV/OmU movies.
//!
//! Generates a clean, dark-themed static HTML site with movie posters,
//! descriptions, and showtimes filtered for English-language screenings.

use anyhow::Result;
use chrono::{DateTime, Utc};
use chrono_tz::Europe::Berlin;
use std::collections::HashMap;
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
    theater_id: i64,
    theater_name: String,
    theater_url: Option<String>,
    showtime: DateTime<Utc>,
    is_ov: bool,
    is_omu: bool,
    is_english_subs: bool,
    is_3d: bool,
    screening_type: Option<String>,
    booking_url: Option<String>,
}

/// Theater data for filtering.
#[derive(Debug)]
#[allow(dead_code)]
struct TheaterInfo {
    id: i64,
    name: String,
    url: Option<String>,
    source: String,
}

/// Generates the static website.
pub fn generate_static_site(db: &Database, output_dir: &str) -> Result<()> {
    let output_path = Path::new(output_dir);

    // Create output directory if it doesn't exist
    fs::create_dir_all(output_path)?;

    // Fetch all data
    let theaters = fetch_theaters(db)?;
    let movies = fetch_ov_movies(db)?;

    // Generate HTML
    let html = generate_html(&movies, &theaters);

    // Write files
    fs::write(output_path.join("index.html"), html)?;
    fs::write(output_path.join("style.css"), generate_css())?;
    fs::write(output_path.join("app.js"), generate_js())?;

    println!(
        "Static site generated: {}/index.html",
        output_path.display()
    );
    println!("  {} movies with OV/OmU screenings", movies.len());
    println!("  {} theaters", theaters.len());

    Ok(())
}

fn fetch_theaters(db: &Database) -> Result<Vec<TheaterInfo>> {
    let conn = db.connection();
    let mut stmt = conn.prepare(
        "SELECT t.id, t.name, t.url, s.name as source
         FROM theaters t
         JOIN sources s ON t.source_id = s.id
         ORDER BY s.name, t.name",
    )?;

    let rows = stmt.query_map([], |row| {
        Ok(TheaterInfo {
            id: row.get(0)?,
            name: row.get(1)?,
            url: row.get(2)?,
            source: row.get(3)?,
        })
    })?;

    let results: Result<Vec<_>, _> = rows.collect();
    Ok(results?)
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
            row.get::<_, Option<i32>>(6)?,
            row.get::<_, Option<i32>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, Option<String>>(11)?,
            row.get::<_, Option<String>>(12)?,
            row.get::<_, Option<i32>>(13)?,
            row.get::<_, Option<String>>(14)?,
            row.get::<_, Option<i32>>(15)?,
            row.get::<_, Option<String>>(16)?,
            row.get::<_, Option<i32>>(17)?,
            row.get::<_, i64>(18)?, // group_key for fetching all related movies
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
                "SELECT t.id, t.name, t.url, s.showtime, s.is_ov, s.is_omu, s.is_english_subs, s.is_3d,
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
                let showtime_str: String = row.get(3)?;
                let showtime = DateTime::parse_from_rfc3339(&showtime_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or(now);

                Ok(ScreeningData {
                    theater_id: row.get(0)?,
                    theater_name: row.get(1)?,
                    theater_url: row.get(2)?,
                    showtime,
                    is_ov: row.get::<_, i32>(4)? != 0,
                    is_omu: row.get::<_, i32>(5)? != 0,
                    is_english_subs: row.get::<_, i32>(6)? != 0,
                    is_3d: row.get::<_, i32>(7)? != 0,
                    screening_type: row.get(8)?,
                    booking_url: row.get(9)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            // No TMDB ID - just get screenings for this specific movie
            let mut stmt = conn.prepare(
                "SELECT t.id, t.name, t.url, s.showtime, s.is_ov, s.is_omu, s.is_english_subs, s.is_3d,
                        s.screening_type, s.booking_url
                 FROM screenings s
                 JOIN theaters t ON s.theater_id = t.id
                 WHERE s.movie_id = ?1
                   AND (s.is_ov = 1 OR s.is_omu = 1 OR s.is_english_subs = 1)
                   AND s.showtime >= ?2
                 ORDER BY s.showtime",
            )?;
            let rows = stmt.query_map([&id.to_string(), &now.to_rfc3339()], |row| {
                let showtime_str: String = row.get(3)?;
                let showtime = DateTime::parse_from_rfc3339(&showtime_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or(now);

                Ok(ScreeningData {
                    theater_id: row.get(0)?,
                    theater_name: row.get(1)?,
                    theater_url: row.get(2)?,
                    showtime,
                    is_ov: row.get::<_, i32>(4)? != 0,
                    is_omu: row.get::<_, i32>(5)? != 0,
                    is_english_subs: row.get::<_, i32>(6)? != 0,
                    is_3d: row.get::<_, i32>(7)? != 0,
                    screening_type: row.get(8)?,
                    booking_url: row.get(9)?,
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

fn generate_html(movies: &[MovieData], theaters: &[TheaterInfo]) -> String {
    let mut html = String::new();

    // DOCTYPE and head
    html.push_str(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Film Finder - OV Movies in Berlin</title>
    <link rel="stylesheet" href="style.css">
    <link rel="preconnect" href="https://fonts.googleapis.com">
    <link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
    <link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600&display=swap" rel="stylesheet">
</head>
<body>
    <header class="site-header">
        <h1>Film Finder</h1>
        <p class="tagline">English-language screenings in Berlin</p>
    </header>

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

    // Group theaters by source
    let mut theaters_by_source: HashMap<String, Vec<&TheaterInfo>> = HashMap::new();
    for theater in theaters {
        theaters_by_source
            .entry(theater.source.clone())
            .or_default()
            .push(theater);
    }

    for (source, source_theaters) in &theaters_by_source {
        html.push_str(&format!(
            r#"                    <li class="theater-group">
                        <strong>{}</strong>
                        <ul>
"#,
            escape_html(source)
        ));

        for theater in source_theaters {
            html.push_str(&format!(
                r#"                            <li>
                                <label>
                                    <input type="checkbox" name="theater" value="{}" checked>
                                    {}
                                </label>
                            </li>
"#,
                theater.id,
                escape_html(&theater.name)
            ));
        }

        html.push_str(
            r#"                        </ul>
                    </li>
"#,
        );
    }

    html.push_str(
        r#"                </ul>
            </fieldset>
        </details>
        <div class="filter-controls">
            <div class="search-box">
                <input type="search" id="search" placeholder="Search movies, directors, writers..." autocomplete="off">
            </div>
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
            format!(
                r#"<span class="no-poster">{}</span>"#,
                escape_html(&display_title.chars().next().unwrap_or('?').to_string())
            )
        };

        // Collect theater IDs for this movie
        let theater_ids: Vec<String> = movie
            .screenings
            .iter()
            .map(|s| s.theater_id.to_string())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let theater_ids_str = theater_ids.join(",");

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

        // Determine if we should show the original title (when different from display)
        let original_title_display = movie.original_title.as_ref().and_then(|orig| {
            if orig != display_title {
                Some(orig.as_str())
            } else {
                None
            }
        });

        html.push_str(&format!(
            r#"        <article class="movie-card" data-movie-id="{}" data-theaters="{}" data-english="{}" data-search="{}">
            <figure class="movie-poster">
                {}
            </figure>
            <header class="movie-header">
                <h2 class="movie-title-row">{}{}{}</h2>
"#,
            movie.id,
            theater_ids_str,
            is_english,
            escape_html(&search_text),
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
                let truncated = if overview.len() > 200 {
                    format!("{}...", &overview[..200])
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
                r#"                <div class="screening-day">
                    <h4>{}</h4>
                    <ul class="screening-times">
"#,
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
                    screening.theater_id,
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

    html.push_str(
        r#"    </main>

    <footer class="site-footer">
        <p>Data from UCI Kinowelt, CineStar, and Yorck cinemas. Movie info from TMDB.</p>
    </footer>

    <script src="app.js"></script>
</body>
</html>
"#,
    );

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
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 0.125rem 0.5rem;
    font-size: 0.8125rem;
}

.movie-crew dt {
    color: var(--text-muted);
    font-weight: 500;
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
        grid-template-columns: auto 1fr;
        gap: 0.5rem;
    }

    .screening-tags,
    .screening a {
        grid-column: 1 / -1;
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
    const selectAllBtn = document.getElementById('select-all');
    const selectNoneBtn = document.getElementById('select-none');
    const showAllOvCheckbox = document.getElementById('show-all-ov');
    const searchInput = document.getElementById('search');

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

    // Filter functionality (theaters + English/all OV + search)
    function updateMoviesVisibility() {
        const selectedTheaters = new Set(
            Array.from(theaterCheckboxes)
                .filter(cb => cb.checked)
                .map(cb => cb.value)
        );
        const showAllOv = showAllOvCheckbox?.checked || false;
        const searchQuery = (searchInput?.value || '').toLowerCase().trim();

        movieCards.forEach(card => {
            const movieTheaters = card.dataset.theaters.split(',');
            const hasSelectedTheater = movieTheaters.some(t => selectedTheaters.has(t));
            const isEnglish = card.dataset.english === 'true';
            const searchText = card.dataset.search || '';

            // Check search match
            const matchesSearch = searchQuery === '' || searchText.includes(searchQuery);

            // Default is English only; if "Show all OV" is checked, show all
            const matchesLanguage = showAllOv || isEnglish;

            // Hide if no selected theater OR doesn't match language filter OR doesn't match search
            const shouldHide = !hasSelectedTheater || !matchesLanguage || !matchesSearch;
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

        document.querySelectorAll('.screening').forEach(screening => {
            const theaterId = screening.dataset.theater;
            screening.classList.toggle('hidden', !selectedTheaters.has(theaterId));
        });
    }

    theaterCheckboxes.forEach(cb => {
        cb.addEventListener('change', updateMoviesVisibility);
    });

    showAllOvCheckbox?.addEventListener('change', updateMoviesVisibility);

    searchInput?.addEventListener('input', updateMoviesVisibility);

    selectAllBtn?.addEventListener('click', () => {
        theaterCheckboxes.forEach(cb => cb.checked = true);
        updateMoviesVisibility();
    });

    selectNoneBtn?.addEventListener('click', () => {
        theaterCheckboxes.forEach(cb => cb.checked = false);
        updateMoviesVisibility();
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

    // Initial filter (English only by default)
    updateMoviesVisibility();
});
"#
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
