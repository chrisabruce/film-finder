//! Database module for storing movie showtimes.
//!
//! Uses SQLite for persistent storage. Each scrape replaces old data
//! for that source to keep the schedule current.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};

use crate::models::{Movie, Screening, Theater};

/// Database wrapper for movie showtime storage.
pub struct Database {
    conn: Connection,
}

impl Database {
    /// Opens or creates the database at the given path.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Database { conn };
        db.init_schema()?;
        Ok(db)
    }

    /// Returns a reference to the underlying connection.
    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    /// Creates required tables if they don't exist.
    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            -- Source websites (UCI, Yorck, etc.)
            CREATE TABLE IF NOT EXISTS sources (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                url TEXT NOT NULL,
                last_scraped TEXT
            );

            -- Theaters/cinema locations
            CREATE TABLE IF NOT EXISTS theaters (
                id INTEGER PRIMARY KEY,
                source_id INTEGER NOT NULL,
                external_id TEXT NOT NULL,
                name TEXT NOT NULL,
                city TEXT,
                address TEXT,
                url TEXT,
                latitude REAL,
                longitude REAL,
                FOREIGN KEY (source_id) REFERENCES sources(id),
                UNIQUE (source_id, external_id)
            );

            -- Movies
            CREATE TABLE IF NOT EXISTS movies (
                id INTEGER PRIMARY KEY,
                source_id INTEGER NOT NULL,
                external_id TEXT,
                title TEXT NOT NULL,
                runtime_minutes INTEGER,
                rating TEXT,
                -- TMDB enrichment fields
                tmdb_id INTEGER,
                english_title TEXT,
                original_title TEXT,
                german_title TEXT,
                original_language TEXT,
                year INTEGER,
                genres TEXT,
                overview TEXT,
                poster_url TEXT,
                tmdb_url TEXT,
                director TEXT,
                director_id INTEGER,
                writer TEXT,
                writer_id INTEGER,
                cinematographer TEXT,
                cinematographer_id INTEGER,
                FOREIGN KEY (source_id) REFERENCES sources(id),
                UNIQUE (source_id, external_id)
            );

            -- Individual screenings
            CREATE TABLE IF NOT EXISTS screenings (
                id INTEGER PRIMARY KEY,
                movie_id INTEGER NOT NULL,
                theater_id INTEGER NOT NULL,
                showtime TEXT NOT NULL,
                screening_type TEXT,
                is_ov INTEGER DEFAULT 0,
                is_omu INTEGER DEFAULT 0,
                is_english_subs INTEGER DEFAULT 0,
                is_3d INTEGER DEFAULT 0,
                booking_url TEXT,
                FOREIGN KEY (movie_id) REFERENCES movies(id),
                FOREIGN KEY (theater_id) REFERENCES theaters(id)
            );

            -- Index for fast OV/OmU lookups
            CREATE INDEX IF NOT EXISTS idx_screenings_ov
                ON screenings(is_ov, is_omu, is_english_subs);
            CREATE INDEX IF NOT EXISTS idx_screenings_showtime
                ON screenings(showtime);
            ",
        )?;
        Ok(())
    }

    /// Gets or creates a source entry, returns its ID.
    pub fn get_or_create_source(&self, name: &str, url: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT OR IGNORE INTO sources (name, url) VALUES (?1, ?2)",
            params![name, url],
        )?;

        let id: i64 = self.conn.query_row(
            "SELECT id FROM sources WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )?;

        Ok(id)
    }

    /// Updates the last scraped timestamp for a source.
    pub fn update_source_timestamp(&self, source_id: i64) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE sources SET last_scraped = ?1 WHERE id = ?2",
            params![now, source_id],
        )?;
        Ok(())
    }

    /// Removes all data for a source (theaters, movies, screenings).
    /// Called before re-scraping to ensure fresh data.
    pub fn clear_source_data(&self, source_id: i64) -> Result<()> {
        // Delete screenings first (foreign key constraints)
        self.conn.execute(
            "DELETE FROM screenings WHERE movie_id IN
             (SELECT id FROM movies WHERE source_id = ?1)",
            params![source_id],
        )?;
        self.conn.execute(
            "DELETE FROM movies WHERE source_id = ?1",
            params![source_id],
        )?;
        self.conn.execute(
            "DELETE FROM theaters WHERE source_id = ?1",
            params![source_id],
        )?;
        Ok(())
    }

    /// Inserts a theater, returns its database ID.
    pub fn insert_theater(&self, source_id: i64, theater: &Theater) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO theaters (source_id, external_id, name, city, address, url, latitude, longitude)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                source_id,
                theater.external_id,
                theater.name,
                theater.city,
                theater.address,
                theater.url,
                theater.latitude,
                theater.longitude
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Inserts a movie or returns existing ID if already present.
    pub fn insert_movie(&self, source_id: i64, movie: &Movie) -> Result<i64> {
        // Try to insert, ignore if already exists
        self.conn.execute(
            "INSERT OR IGNORE INTO movies (source_id, external_id, title, runtime_minutes, rating)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                source_id,
                movie.external_id,
                movie.title,
                movie.runtime_minutes,
                movie.rating
            ],
        )?;

        // Get the ID (either newly inserted or existing)
        let id: i64 = if let Some(ref ext_id) = movie.external_id {
            self.conn.query_row(
                "SELECT id FROM movies WHERE source_id = ?1 AND external_id = ?2",
                params![source_id, ext_id],
                |row| row.get(0),
            )?
        } else {
            // For movies without external_id, use title match
            self.conn.query_row(
                "SELECT id FROM movies WHERE source_id = ?1 AND title = ?2",
                params![source_id, movie.title],
                |row| row.get(0),
            )?
        };

        Ok(id)
    }

    /// Inserts a screening.
    pub fn insert_screening(
        &self,
        movie_id: i64,
        theater_id: i64,
        screening: &Screening,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO screenings
             (movie_id, theater_id, showtime, screening_type, is_ov, is_omu, is_english_subs, is_3d, booking_url)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                movie_id,
                theater_id,
                screening.showtime.to_rfc3339(),
                screening.screening_type,
                screening.is_ov,
                screening.is_omu,
                screening.is_english_subs,
                screening.is_3d,
                screening.booking_url
            ],
        )?;
        Ok(())
    }

    /// Updates a movie with TMDB data.
    #[allow(clippy::too_many_arguments)]
    pub fn update_movie_tmdb(
        &self,
        movie_id: i64,
        tmdb_id: i32,
        english_title: &str,
        original_title: &str,
        german_title: Option<&str>,
        original_language: &str,
        year: Option<i32>,
        genres: &str,
        overview: &str,
        poster_url: Option<&str>,
        director: Option<&str>,
        director_id: Option<i32>,
        writer: Option<&str>,
        writer_id: Option<i32>,
        cinematographer: Option<&str>,
        cinematographer_id: Option<i32>,
    ) -> Result<()> {
        let tmdb_url = format!("https://www.themoviedb.org/movie/{}", tmdb_id);
        self.conn.execute(
            "UPDATE movies SET tmdb_id = ?1, english_title = ?2, original_title = ?3, german_title = ?4,
             original_language = ?5, year = ?6, genres = ?7, overview = ?8, poster_url = ?9, tmdb_url = ?10,
             director = ?11, director_id = ?12, writer = ?13, writer_id = ?14,
             cinematographer = ?15, cinematographer_id = ?16
             WHERE id = ?17",
            params![
                tmdb_id,
                english_title,
                original_title,
                german_title,
                original_language,
                year,
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
                movie_id
            ],
        )?;
        Ok(())
    }

    /// Gets all movies that haven't been enriched with TMDB data yet.
    pub fn get_movies_without_tmdb(&self) -> Result<Vec<(i64, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, title FROM movies WHERE tmdb_id IS NULL")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let results: Result<Vec<_>, _> = rows.collect();
        Ok(results?)
    }

    /// Gets all unique movies with screening counts.
    pub fn get_movies(&self) -> Result<Vec<MovieInfo>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.title, m.original_title, m.year, m.genres, m.overview,
                    m.poster_url, m.tmdb_url, m.runtime_minutes,
                    COUNT(s.id) as screening_count,
                    SUM(CASE WHEN s.is_ov = 1 OR s.is_omu = 1 OR s.is_english_subs = 1 THEN 1 ELSE 0 END) as ov_count
             FROM movies m
             LEFT JOIN screenings s ON m.id = s.movie_id
             GROUP BY LOWER(COALESCE(m.original_title, m.title))
             ORDER BY screening_count DESC, m.title",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(MovieInfo {
                title: row.get(0)?,
                original_title: row.get(1)?,
                year: row.get(2)?,
                genres: row.get(3)?,
                overview: row.get(4)?,
                poster_url: row.get(5)?,
                tmdb_url: row.get(6)?,
                runtime_minutes: row.get(7)?,
                screening_count: row.get(8)?,
                ov_count: row.get(9)?,
            })
        })?;

        let results: Result<Vec<_>, _> = rows.collect();
        Ok(results?)
    }

    /// Finds all OV (Original Version) screenings, optionally filtered by city.
    pub fn find_ov_screenings(&self, city: Option<&str>) -> Result<Vec<ScreeningResult>> {
        let mut query = String::from(
            "SELECT m.title, m.runtime_minutes, t.name, t.city,
                    s.showtime, s.screening_type, s.is_ov, s.is_omu, s.is_english_subs, s.is_3d, s.booking_url
             FROM screenings s
             JOIN movies m ON s.movie_id = m.id
             JOIN theaters t ON s.theater_id = t.id
             WHERE (s.is_ov = 1 OR s.is_omu = 1 OR s.is_english_subs = 1)",
        );

        if city.is_some() {
            query.push_str(" AND t.city = ?1");
        }
        query.push_str(" ORDER BY s.showtime, m.title");

        let mut stmt = self.conn.prepare(&query)?;

        let rows = if let Some(city) = city {
            stmt.query_map(params![city], row_to_screening_result)?
        } else {
            stmt.query_map([], row_to_screening_result)?
        };

        let results: Result<Vec<_>, _> = rows.collect();
        Ok(results?)
    }

    /// Gets all upcoming screenings.
    pub fn get_all_screenings(&self) -> Result<Vec<ScreeningResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.title, m.runtime_minutes, t.name, t.city,
                    s.showtime, s.screening_type, s.is_ov, s.is_omu, s.is_english_subs, s.is_3d, s.booking_url
             FROM screenings s
             JOIN movies m ON s.movie_id = m.id
             JOIN theaters t ON s.theater_id = t.id
             ORDER BY s.showtime, m.title",
        )?;

        let rows = stmt.query_map([], row_to_screening_result)?;
        let results: Result<Vec<_>, _> = rows.collect();
        Ok(results?)
    }
}

/// Movie information for the films command.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MovieInfo {
    pub title: String,
    pub original_title: Option<String>,
    pub year: Option<i32>,
    pub genres: Option<String>,
    pub overview: Option<String>,
    pub poster_url: Option<String>,
    pub tmdb_url: Option<String>,
    pub runtime_minutes: Option<i32>,
    pub screening_count: i32,
    pub ov_count: i32,
}

/// A screening result with denormalized movie and theater info.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Fields kept for future features (filtering by city, booking links)
pub struct ScreeningResult {
    pub movie_title: String,
    pub runtime_minutes: Option<i32>,
    pub theater_name: String,
    pub city: Option<String>,
    pub showtime: DateTime<Utc>,
    pub screening_type: Option<String>,
    pub is_ov: bool,
    pub is_omu: bool,
    pub is_english_subs: bool,
    pub is_3d: bool,
    pub booking_url: Option<String>,
}

fn row_to_screening_result(row: &rusqlite::Row) -> rusqlite::Result<ScreeningResult> {
    let showtime_str: String = row.get(4)?;
    let showtime = DateTime::parse_from_rfc3339(&showtime_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    Ok(ScreeningResult {
        movie_title: row.get(0)?,
        runtime_minutes: row.get(1)?,
        theater_name: row.get(2)?,
        city: row.get(3)?,
        showtime,
        screening_type: row.get(5)?,
        is_ov: row.get::<_, i32>(6)? != 0,
        is_omu: row.get::<_, i32>(7)? != 0,
        is_english_subs: row.get::<_, i32>(8)? != 0,
        is_3d: row.get::<_, i32>(9)? != 0,
        booking_url: row.get(10)?,
    })
}
