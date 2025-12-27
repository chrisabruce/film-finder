//! Film Finder - Find English (OV/OmU) movie screenings in Berlin.
//!
//! This application scrapes movie showtime data from cinema websites
//! and stores it in a local SQLite database for easy querying.
//!
//! Usage:
//!   film-finder scrape     # Update the database with latest showtimes
//!   film-finder films      # List all movies with screening counts
//!   film-finder list       # Show all upcoming screenings
//!   film-finder ov         # Show only OV/OmU (English) screenings
//!   film-finder search <query>  # Search for a specific movie
//!   film-finder db-reset   # Delete the database and start fresh

mod db;
mod models;
mod scraper;
mod scrapers;
mod static_site;
mod tmdb;

use anyhow::Result;
use chrono::Utc;
use chrono_tz::Europe::Berlin;

use crate::db::Database;
use crate::scraper::Scraper;
use crate::scrapers::{CineStarScraper, UciScraper, YorckScraper};
use crate::static_site::generate_static_site;
use crate::tmdb::{load_api_key, TmdbClient};

const DB_PATH: &str = "film-finder.db";

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match command {
        "scrape" => cmd_scrape().await?,
        "films" => cmd_films()?,
        "list" => cmd_list(None)?,
        "ov" => cmd_ov()?,
        "search" => {
            let mut query = "";
            let mut ov_only = false;
            for arg in args.iter().skip(2) {
                if arg == "--ov" || arg == "-o" {
                    ov_only = true;
                } else if query.is_empty() {
                    query = arg;
                }
            }
            cmd_search(query, ov_only)?;
        }
        "static" => {
            // Output path: CLI arg > .env > default
            let output_dir = args.get(2).cloned().unwrap_or_else(|| {
                std::env::var("STATIC_OUTPUT_DIR").unwrap_or_else(|_| "html".to_string())
            });
            cmd_static(&output_dir)?;
        }
        "db-reset" => cmd_db_reset()?,
        "help" | "--help" | "-h" => print_help(),
        _ => {
            eprintln!("Unknown command: {}", command);
            print_help();
        }
    }

    Ok(())
}

fn print_help() {
    println!(
        r#"Film Finder - Find English (OV/OmU) movies in Berlin

USAGE:
    film-finder <COMMAND>

COMMANDS:
    scrape          Fetch latest showtimes from all sources
    films           List all movies with screening counts
    list            Show all upcoming screenings
    ov              Show only OV/OmU (Original Version) screenings
    search <query> [--ov]  Search for a movie by title (--ov for OV only)
    static [path]   Generate static website (default: html/)
    db-reset        Delete the database and start fresh
    help            Show this help message

EXAMPLES:
    film-finder scrape
    film-finder films
    film-finder ov
    film-finder search avatar
    film-finder static ./public
"#
    );
}

/// Scrapes all configured sources and updates the database.
async fn cmd_scrape() -> Result<()> {
    println!("Film Finder - Updating showtime data...\n");

    let db = Database::open(DB_PATH)?;

    // Add more scrapers here as they're implemented
    let scrapers: Vec<Box<dyn Scraper>> = vec![
        Box::new(UciScraper::new()),
        Box::new(CineStarScraper::new()),
        Box::new(YorckScraper::new()),
    ];

    for scraper in &scrapers {
        println!("=== {} ===", scraper.name());

        // Get or create source
        let source_id = db.get_or_create_source(scraper.name(), scraper.url())?;

        // Clear old data for this source
        db.clear_source_data(source_id)?;

        // Scrape fresh data
        match scraper.scrape().await {
            Ok(theater_data) => {
                let mut total_movies = 0;
                let mut total_screenings = 0;
                let mut ov_screenings = 0;

                for data in theater_data {
                    let theater_id = db.insert_theater(source_id, &data.theater)?;

                    for movie_data in &data.movies {
                        let movie_id = db.insert_movie(source_id, &movie_data.movie)?;

                        for screening in &movie_data.screenings {
                            db.insert_screening(movie_id, theater_id, screening)?;
                            total_screenings += 1;
                            if screening.is_ov || screening.is_omu || screening.is_english_subs {
                                ov_screenings += 1;
                            }
                        }

                        total_movies += 1;
                    }
                }

                db.update_source_timestamp(source_id)?;
                println!(
                    "Imported: {} movies, {} screenings ({} OV/OmU)\n",
                    total_movies, total_screenings, ov_screenings
                );
            }
            Err(e) => {
                eprintln!("Error scraping {}: {}", scraper.name(), e);
            }
        }
    }

    // Enrich movies with TMDB data
    enrich_with_tmdb(&db).await?;

    println!("Done! Database updated at {}", DB_PATH);
    Ok(())
}

/// Enriches movies with TMDB metadata.
async fn enrich_with_tmdb(db: &Database) -> Result<()> {
    let api_key = match load_api_key() {
        Ok(key) => key,
        Err(e) => {
            println!("Skipping TMDB enrichment: {}", e);
            return Ok(());
        }
    };

    let movies = db.get_movies_without_tmdb()?;
    if movies.is_empty() {
        return Ok(());
    }

    println!("=== TMDB Enrichment ===");
    println!("Fetching metadata for {} movies...", movies.len());

    let client = TmdbClient::new(api_key);

    // Fetch genre list for mapping IDs to names
    let genre_map = client.fetch_genres().await?;

    let mut enriched = 0;
    let mut not_found = 0;

    for (movie_id, title) in movies {
        match client.lookup_movie(&title, &genre_map).await {
            Ok(Some(tmdb)) => {
                db.update_movie_tmdb(
                    movie_id,
                    tmdb.tmdb_id,
                    &tmdb.english_title,
                    &tmdb.original_title,
                    tmdb.german_title.as_deref(),
                    &tmdb.original_language,
                    tmdb.year,
                    &tmdb.genres,
                    &tmdb.overview,
                    tmdb.poster_url.as_deref(),
                    tmdb.director.as_deref(),
                    tmdb.director_id,
                    tmdb.writer.as_deref(),
                    tmdb.writer_id,
                    tmdb.cinematographer.as_deref(),
                    tmdb.cinematographer_id,
                )?;
                enriched += 1;
            }
            Ok(None) => {
                not_found += 1;
            }
            Err(e) => {
                eprintln!("Error looking up '{}': {}", title, e);
            }
        }
    }

    println!(
        "Enriched {} movies, {} not found on TMDB\n",
        enriched, not_found
    );

    Ok(())
}

/// Lists all movies with screening counts.
fn cmd_films() -> Result<()> {
    let db = Database::open(DB_PATH)?;
    let movies = db.get_movies()?;

    if movies.is_empty() {
        println!("No films found. Run 'film-finder scrape' first.");
        return Ok(());
    }

    println!("=== Films Currently Showing ===\n");

    for m in &movies {
        // Use original title if available, otherwise the scraped title
        let display_title = m.original_title.as_ref().unwrap_or(&m.title);

        // Year and runtime info
        let year_str = m.year.map(|y| format!(" ({})", y)).unwrap_or_default();
        let runtime_str = m
            .runtime_minutes
            .map(|r| format!(" - {}min", r))
            .unwrap_or_default();

        // Screening counts
        let ov_indicator = if m.ov_count > 0 {
            format!(" [{} OV]", m.ov_count)
        } else {
            String::new()
        };

        println!(
            "{}{}{} - {} screenings{}",
            display_title, year_str, runtime_str, m.screening_count, ov_indicator
        );

        // Show genres if available
        if let Some(ref genres) = m.genres {
            if !genres.is_empty() {
                println!("  Genres: {}", genres);
            }
        }

        // Show TMDB link if available
        if let Some(ref url) = m.tmdb_url {
            println!("  TMDB: {}", url);
        }

        println!();
    }

    println!("{} films total", movies.len());
    Ok(())
}

/// Deletes the database file.
fn cmd_db_reset() -> Result<()> {
    if std::path::Path::new(DB_PATH).exists() {
        std::fs::remove_file(DB_PATH)?;
        println!("Database deleted: {}", DB_PATH);
    } else {
        println!("No database file found at {}", DB_PATH);
    }
    Ok(())
}

/// Generates a static website with OV/OmU movies.
fn cmd_static(output_dir: &str) -> Result<()> {
    let db = Database::open(DB_PATH)?;
    generate_static_site(&db, output_dir)?;
    Ok(())
}

/// Lists all upcoming screenings.
fn cmd_list(filter_movie: Option<&str>) -> Result<()> {
    cmd_list_filtered(filter_movie, false)
}

/// Lists upcoming screenings with optional movie and OV filters.
fn cmd_list_filtered(filter_movie: Option<&str>, ov_only: bool) -> Result<()> {
    let db = Database::open(DB_PATH)?;
    let screenings = db.get_all_screenings()?;

    if screenings.is_empty() {
        println!("No screenings found. Run 'film-finder scrape' first.");
        return Ok(());
    }

    let now = Utc::now();
    let mut count = 0;

    for s in screenings {
        // Skip past screenings
        if s.showtime < now {
            continue;
        }

        // Filter by movie title if specified
        if let Some(query) = filter_movie {
            if !s.movie_title.to_lowercase().contains(&query.to_lowercase()) {
                continue;
            }
        }

        // Filter for OV only if requested
        if ov_only && !s.is_ov && !s.is_omu && !s.is_english_subs {
            continue;
        }

        print_screening(&s);
        count += 1;
    }

    println!("\n{} upcoming screenings", count);
    Ok(())
}

/// Lists only OV/OmU screenings.
fn cmd_ov() -> Result<()> {
    let db = Database::open(DB_PATH)?;
    let screenings = db.find_ov_screenings(None)?;

    if screenings.is_empty() {
        println!("No OV/OmU screenings found. Run 'film-finder scrape' first.");
        return Ok(());
    }

    let now = Utc::now();
    let mut count = 0;
    let mut current_date = String::new();

    println!("=== OV/OmU Screenings (Original Version / English) ===\n");

    for s in screenings {
        // Skip past screenings
        if s.showtime < now {
            continue;
        }

        // Print date header when it changes
        let local_time = s.showtime.with_timezone(&Berlin);
        let date_str = local_time.format("%A, %B %d").to_string();
        if date_str != current_date {
            if !current_date.is_empty() {
                println!();
            }
            println!("--- {} ---", date_str);
            current_date = date_str;
        }

        print_screening(&s);
        count += 1;
    }

    println!("\n{} OV/OmU screenings found", count);
    Ok(())
}

/// Searches for a movie by title.
fn cmd_search(query: &str, ov_only: bool) -> Result<()> {
    if query.is_empty() {
        println!("Usage: film-finder search <movie title> [--ov]");
        return Ok(());
    }

    if ov_only {
        println!("Searching for: {} (OV only)\n", query);
        cmd_list_filtered(Some(query), true)
    } else {
        println!("Searching for: {}\n", query);
        cmd_list_filtered(Some(query), false)
    }
}

/// Prints a single screening in a readable format.
fn print_screening(s: &db::ScreeningResult) {
    let local_time = s.showtime.with_timezone(&Berlin);
    let time_str = local_time.format("%H:%M").to_string();

    // Build format tags
    let mut tags = Vec::new();
    if s.is_ov {
        tags.push("OV");
    }
    if s.is_omu {
        tags.push("OmU");
    }
    if s.is_english_subs {
        tags.push("OmeU");
    }
    if s.is_3d {
        tags.push("3D");
    }
    if let Some(ref t) = s.screening_type {
        tags.push(t);
    }

    let tags_str = if tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", tags.join(", "))
    };

    let runtime_str = s
        .runtime_minutes
        .map(|m| format!(" ({}min)", m))
        .unwrap_or_default();

    println!(
        "  {} - {}{}{} @ {}",
        time_str, s.movie_title, runtime_str, tags_str, s.theater_name
    );
}
