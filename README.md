# film-finder

Find English-language (OV/OmU) movie screenings in Berlin.

I don't speak German, so finding movies in their original language is a pain. This scrapes cinema websites and dumps everything into a local SQLite database so I can quickly see what's playing in English.

## Usage

```bash
# Fetch latest showtimes (also pulls metadata from TMDB)
film-finder scrape

# List all films with screening counts
film-finder films

# Show OV/OmU screenings only
film-finder ov

# Search for a specific movie
film-finder search avatar

# Show everything
film-finder list

# Start fresh
film-finder db-reset
```

## TMDB Integration

The scraper can pull additional metadata from TMDB (The Movie Database) to get normalized English titles, release years, genres, and overviews. German releases sometimes have different names, so this helps identify what movies actually are.

To enable it:

1. Get a free API key from https://www.themoviedb.org/settings/api
2. Create a `.env` file: `echo 'TMDB_API_KEY=your_key_here' > .env`

If no API key is set, scraping still works - you just won't get the extra metadata.

## What it scrapes

- UCI Kinowelt (Berlin Eastgate, East Side Gallery, Gropius Passagen, Potsdam)
- CineStar (CUBIX Alexanderplatz, Treptower Park, Tegel, Hellersdorf, KulturBrauerei)
- Yorck (Babylon Kreuzberg, Capitol Dahlem, Cinema Paris, Delphi Filmpalast, delphi LUX, Filmtheater am Friedrichshain, Kant Kino, Neues Off, Odeon, Passage, Rollberg, Yorck, Blauer Stern)

The data gets stored in `film-finder.db` in the current directory.

OV = Original Version (no dubbing)
OmU = Original mit Untertiteln (original with German subtitles)
OmeU = Original mit englischen Untertiteln (original with English subtitles)

## Building

```bash
cargo build --release
```

Needs Rust 1.70+ (uses edition 2021).

## Adding more cinemas

The scraper is set up to be extensible. To add a new cinema chain:

1. Create `src/scrapers/whatever.rs`
2. Implement the `Scraper` trait (see `uci.rs` for an example)
3. Add it to the scraper list in `main.rs`

The main things you need to figure out for each site:
- How to get the program page for each theater
- How movie containers are structured in the HTML
- Where the showtime links are and how dates/times are encoded
- How OV screenings are marked (usually a CSS class or legend)

## Why Rust?

Async HTTP + HTML parsing is well-supported, SQLite bindings are solid, and it compiles to a single binary I can just copy around.
