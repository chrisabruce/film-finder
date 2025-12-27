//! TMDB API client for movie metadata enrichment.
//!
//! Uses the TMDB API to fetch normalized titles, release years,
//! genres, overviews, poster URLs, and crew information.

use anyhow::{anyhow, Result};
use serde::Deserialize;
use std::collections::HashMap;

const TMDB_API_BASE: &str = "https://api.themoviedb.org/3";
const TMDB_IMAGE_BASE: &str = "https://image.tmdb.org/t/p/w500";

/// TMDB API client.
pub struct TmdbClient {
    api_key: String,
    client: reqwest::Client,
}

/// Movie search result from TMDB.
#[derive(Debug, Deserialize)]
struct MovieSearchResult {
    id: i32,
    title: String,
    original_title: String,
    original_language: String,
    release_date: Option<String>,
    overview: Option<String>,
    poster_path: Option<String>,
    genre_ids: Vec<i32>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<MovieSearchResult>,
}

/// Genre information from TMDB.
#[derive(Debug, Deserialize)]
struct Genre {
    id: i32,
    name: String,
}

#[derive(Debug, Deserialize)]
struct GenreListResponse {
    genres: Vec<Genre>,
}

/// Enriched movie data from TMDB.
#[derive(Debug, Clone)]
pub struct TmdbMovie {
    pub tmdb_id: i32,
    pub english_title: String,
    pub original_title: String,
    pub german_title: Option<String>,
    pub original_language: String,
    pub year: Option<i32>,
    pub genres: String,
    pub overview: String,
    pub poster_url: Option<String>,
    pub director: Option<String>,
    pub director_id: Option<i32>,
    pub writer: Option<String>,
    pub writer_id: Option<i32>,
    pub cinematographer: Option<String>,
    pub cinematographer_id: Option<i32>,
}

/// Credits response from TMDB.
#[derive(Debug, Deserialize)]
struct CreditsResponse {
    crew: Vec<CrewMember>,
}

/// Crew member from TMDB credits.
#[derive(Debug, Deserialize)]
struct CrewMember {
    id: i32,
    name: String,
    job: String,
}

/// Movie details for fetching localized titles.
#[derive(Debug, Deserialize)]
struct MovieDetails {
    title: String,
}

impl TmdbClient {
    /// Creates a new TMDB client with the given API key.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
        }
    }

    /// Fetches the genre list from TMDB (for mapping genre IDs to names).
    pub async fn fetch_genres(&self) -> Result<HashMap<i32, String>> {
        let url = format!(
            "{}/genre/movie/list?api_key={}&language=en-US",
            TMDB_API_BASE, self.api_key
        );

        let resp: GenreListResponse = self.client.get(&url).send().await?.json().await?;
        Ok(resp.genres.into_iter().map(|g| (g.id, g.name)).collect())
    }

    /// Searches for a movie by title.
    async fn search_movie(&self, title: &str) -> Result<Option<MovieSearchResult>> {
        let url = format!(
            "{}/search/movie?api_key={}&query={}&language=en-US&page=1",
            TMDB_API_BASE,
            self.api_key,
            urlencoding::encode(title)
        );

        let resp: SearchResponse = self.client.get(&url).send().await?.json().await?;
        Ok(resp.results.into_iter().next())
    }

    /// Fetches movie credits (director, writer, cinematographer) with their TMDB IDs.
    async fn fetch_credits(
        &self,
        movie_id: i32,
    ) -> Result<(
        Option<String>,
        Option<i32>,
        Option<String>,
        Option<i32>,
        Option<String>,
        Option<i32>,
    )> {
        let url = format!(
            "{}/movie/{}/credits?api_key={}",
            TMDB_API_BASE, movie_id, self.api_key
        );

        let resp: CreditsResponse = self.client.get(&url).send().await?.json().await?;

        // Find directors
        let directors: Vec<&CrewMember> =
            resp.crew.iter().filter(|c| c.job == "Director").collect();
        let director = if directors.is_empty() {
            None
        } else {
            Some(
                directors
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        };
        let director_id = directors.first().map(|c| c.id);

        // Find writers (Screenplay or Writer)
        let writers: Vec<&CrewMember> = resp
            .crew
            .iter()
            .filter(|c| c.job == "Screenplay" || c.job == "Writer")
            .collect();
        // Deduplicate by ID
        let mut seen_ids = std::collections::HashSet::new();
        let unique_writers: Vec<&CrewMember> = writers
            .into_iter()
            .filter(|c| seen_ids.insert(c.id))
            .collect();
        let writer = if unique_writers.is_empty() {
            None
        } else {
            Some(
                unique_writers
                    .iter()
                    .map(|c| c.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        };
        let writer_id = unique_writers.first().map(|c| c.id);

        // Find cinematographer (Director of Photography)
        let cinematographer_crew = resp
            .crew
            .iter()
            .find(|c| c.job == "Director of Photography");
        let cinematographer = cinematographer_crew.map(|c| c.name.clone());
        let cinematographer_id = cinematographer_crew.map(|c| c.id);

        Ok((
            director,
            director_id,
            writer,
            writer_id,
            cinematographer,
            cinematographer_id,
        ))
    }

    /// Fetches the German title for a movie.
    async fn fetch_german_title(&self, movie_id: i32) -> Result<Option<String>> {
        let url = format!(
            "{}/movie/{}?api_key={}&language=de-DE",
            TMDB_API_BASE, movie_id, self.api_key
        );

        let resp: MovieDetails = self.client.get(&url).send().await?.json().await?;
        Ok(Some(resp.title))
    }

    /// Searches for a movie and returns enriched data.
    pub async fn lookup_movie(
        &self,
        title: &str,
        genre_map: &HashMap<i32, String>,
    ) -> Result<Option<TmdbMovie>> {
        let result = match self.search_movie(title).await? {
            Some(r) => r,
            None => return Ok(None),
        };

        // Extract year from release date
        let year = result.release_date.as_ref().and_then(|d| {
            if d.len() >= 4 {
                d[..4].parse().ok()
            } else {
                None
            }
        });

        // Map genre IDs to names
        let genres: Vec<&str> = result
            .genre_ids
            .iter()
            .filter_map(|id| genre_map.get(id).map(|s| s.as_str()))
            .collect();
        let genres_str = genres.join(", ");

        // Build poster URL
        let poster_url = result
            .poster_path
            .map(|p| format!("{}{}", TMDB_IMAGE_BASE, p));

        // Fetch credits and German title concurrently
        let (credits_result, german_result) = tokio::join!(
            self.fetch_credits(result.id),
            self.fetch_german_title(result.id)
        );

        let (director, director_id, writer, writer_id, cinematographer, cinematographer_id) =
            credits_result.unwrap_or((None, None, None, None, None, None));
        let german_title = german_result.ok().flatten();

        Ok(Some(TmdbMovie {
            tmdb_id: result.id,
            english_title: result.title,
            original_title: result.original_title,
            german_title,
            original_language: result.original_language,
            year,
            genres: genres_str,
            overview: result.overview.unwrap_or_default(),
            poster_url,
            director,
            director_id,
            writer,
            writer_id,
            cinematographer,
            cinematographer_id,
        }))
    }
}

/// Loads the TMDB API key from environment variables.
pub fn load_api_key() -> Result<String> {
    // Load .env file if present
    let _ = dotenvy::dotenv();

    std::env::var("TMDB_API_KEY").map_err(|_| {
        anyhow!(
            "TMDB_API_KEY not set. Get an API key from https://www.themoviedb.org/settings/api\n\
             Then set it in a .env file or environment variable:\n\
             echo 'TMDB_API_KEY=your_key_here' > .env"
        )
    })
}
