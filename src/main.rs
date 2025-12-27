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
        "deploy" => cmd_deploy()?,
        "serve" => cmd_serve().await?,
        "stop" => cmd_stop()?,
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
    deploy          Deploy static site to Cloudflare Pages
    serve           Start background service (scrape + static + deploy every N hours)
    stop            Stop the background service
    db-reset        Delete the database and start fresh
    help            Show this help message

EXAMPLES:
    film-finder scrape
    film-finder films
    film-finder ov
    film-finder search avatar
    film-finder static ./public
    film-finder deploy
    film-finder serve           # Run in foreground
    film-finder serve --daemon  # Detach and run in background
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
                    tmdb.production_countries.as_deref(),
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

/// Deploys the static site to Cloudflare Pages using wrangler.
fn cmd_deploy() -> Result<()> {
    use std::process::Command;

    // Load .env file
    let _ = dotenvy::dotenv();

    // Load required environment variables
    let account_id = std::env::var("CLOUDFLARE_ACCOUNT_ID")
        .map_err(|_| anyhow::anyhow!("CLOUDFLARE_ACCOUNT_ID not set in .env"))?;
    let api_token = std::env::var("CLOUDFLARE_API_TOKEN")
        .map_err(|_| anyhow::anyhow!("CLOUDFLARE_API_TOKEN not set in .env"))?;
    let project_name = std::env::var("CLOUDFLARE_PROJECT_NAME")
        .map_err(|_| anyhow::anyhow!("CLOUDFLARE_PROJECT_NAME not set in .env"))?;

    // Get output directory (same logic as static command)
    let output_dir = std::env::var("STATIC_OUTPUT_DIR").unwrap_or_else(|_| "html".to_string());

    // Check that the output directory exists
    if !std::path::Path::new(&output_dir).exists() {
        return Err(anyhow::anyhow!(
            "Output directory '{}' does not exist. Run 'film-finder static' first.",
            output_dir
        ));
    }

    println!("Deploying {} to Cloudflare Pages...", output_dir);
    println!("Project: {}", project_name);

    // Run wrangler via npx
    let status = Command::new("npx")
        .args([
            "wrangler",
            "pages",
            "deploy",
            &output_dir,
            "--project-name",
            &project_name,
        ])
        .env("CLOUDFLARE_ACCOUNT_ID", &account_id)
        .env("CLOUDFLARE_API_TOKEN", &api_token)
        .status();

    match status {
        Ok(exit_status) => {
            if exit_status.success() {
                println!("\nDeployment complete!");
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "Wrangler exited with status: {}",
                    exit_status
                ))
            }
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                Err(anyhow::anyhow!(
                    "npx not found. Please install Node.js: https://nodejs.org/"
                ))
            } else {
                Err(anyhow::anyhow!("Failed to run wrangler: {}", e))
            }
        }
    }
}

/// Runs the update cycle: scrape → static → deploy
async fn run_update_cycle() -> Result<()> {
    println!("Starting update cycle...");

    // Scrape
    if let Err(e) = cmd_scrape().await {
        eprintln!("Scrape failed: {}", e);
        return Err(e);
    }

    // Generate static site
    let output_dir = std::env::var("STATIC_OUTPUT_DIR").unwrap_or_else(|_| "html".to_string());
    if let Err(e) = cmd_static(&output_dir) {
        eprintln!("Static site generation failed: {}", e);
        return Err(e);
    }

    // Deploy
    if let Err(e) = cmd_deploy() {
        eprintln!("Deploy failed: {}", e);
        return Err(e);
    }

    println!("Update cycle complete.");
    Ok(())
}

/// Starts the background service that periodically updates the site.
async fn cmd_serve() -> Result<()> {
    use std::time::Duration;

    // Load .env file
    let _ = dotenvy::dotenv();

    // Check for --daemon flag
    let args: Vec<String> = std::env::args().collect();
    let daemon_mode = args.iter().any(|a| a == "--daemon" || a == "-d");

    // Get update interval from env (default 12 hours)
    let interval_hours: u64 = std::env::var("UPDATE_INTERVAL_HOURS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(12);

    let interval = Duration::from_secs(interval_hours * 60 * 60);

    if daemon_mode {
        // Fork and detach
        daemonize()?;
    }

    println!("Film Finder service started");
    println!("Update interval: {} hours", interval_hours);
    println!("PID: {}", std::process::id());

    // Write PID file for management
    let pid_file = std::env::var("PID_FILE").unwrap_or_else(|_| "film-finder.pid".to_string());
    std::fs::write(&pid_file, std::process::id().to_string())?;

    // Run initial update
    if let Err(e) = run_update_cycle().await {
        eprintln!("Initial update failed: {}", e);
    }

    // Schedule periodic updates
    loop {
        let next_run = Utc::now() + chrono::Duration::seconds(interval.as_secs() as i64);
        println!(
            "Next update scheduled for: {}",
            next_run.with_timezone(&Berlin).format("%Y-%m-%d %H:%M:%S")
        );

        tokio::time::sleep(interval).await;

        if let Err(e) = run_update_cycle().await {
            eprintln!("Update cycle failed: {}", e);
            // Continue running despite errors
        }
    }
}

/// Daemonizes the current process (Unix only).
fn daemonize() -> Result<()> {
    use std::fs::OpenOptions;
    use std::process::{Command, Stdio};

    // Get current executable and args
    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args()
        .skip(1)
        .filter(|a| a != "--daemon" && a != "-d")
        .collect();

    // Get log file path from env (default: film-finder.log)
    let log_file = std::env::var("LOG_FILE").unwrap_or_else(|_| "film-finder.log".to_string());

    // Open log file for appending
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .map_err(|e| anyhow::anyhow!("Failed to open log file {}: {}", log_file, e))?;

    let log_err = log.try_clone()?;

    // Spawn detached child process with output to log file
    let child = Command::new(&exe)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err))
        .spawn()?;

    println!("Daemon started with PID: {}", child.id());
    println!("Logging to: {}", log_file);
    std::process::exit(0);
}

/// Stops the background service by reading the PID file and sending SIGTERM.
fn cmd_stop() -> Result<()> {
    // Load .env file
    let _ = dotenvy::dotenv();

    let pid_file = std::env::var("PID_FILE").unwrap_or_else(|_| "film-finder.pid".to_string());

    // Read PID from file
    let pid_str = match std::fs::read_to_string(&pid_file) {
        Ok(s) => s,
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                println!("No service running (PID file not found: {})", pid_file);
                return Ok(());
            }
            return Err(anyhow::anyhow!("Failed to read PID file: {}", e));
        }
    };

    let pid: i32 = pid_str
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid PID in file: {}", pid_str.trim()))?;

    // Send SIGTERM to the process
    #[cfg(unix)]
    {
        use std::process::Command;
        let status = Command::new("kill").arg(pid.to_string()).status();

        match status {
            Ok(exit) if exit.success() => {
                println!("Stopped service (PID: {})", pid);
                // Remove PID file
                let _ = std::fs::remove_file(&pid_file);
            }
            Ok(_) => {
                // Process might already be dead
                println!("Process {} not running", pid);
                let _ = std::fs::remove_file(&pid_file);
            }
            Err(e) => {
                return Err(anyhow::anyhow!("Failed to stop process: {}", e));
            }
        }
    }

    #[cfg(not(unix))]
    {
        return Err(anyhow::anyhow!(
            "Stop command only supported on Unix systems"
        ));
    }

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
