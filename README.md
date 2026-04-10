# OV Berlin

**https://ovberlin.com**

Find English-language (OV/OmU) movie screenings in Berlin.

I don't speak German, so finding movies in their original language is a pain. This scrapes cinema websites, enriches them with TMDB metadata, and publishes a clean static site to Cloudflare Pages. Updated every 12 hours.

## Quick start

```bash
# Build
cargo build --release

# Scrape showtimes (pulls metadata from TMDB if API key is set)
film-finder scrape

# Generate static site
film-finder static

# Deploy to Cloudflare Pages
film-finder deploy

# Or run the full loop (scrape + static + deploy every 12h)
film-finder serve
```

## Docker

```bash
# Build and run
docker compose up -d

# One-shot scrape
docker compose run --rm film-finder scrape

# Logs
docker compose logs -f
```

The container runs `film-finder serve` in the foreground, persisting the database and generated HTML in a Docker volume.

## Configuration

Create a `.env` file with:

```
# Required for movie metadata (free at https://www.themoviedb.org/settings/api)
TMDB_API_KEY=your_key_here

# Required for deployment
CLOUDFLARE_ACCOUNT_ID=your_account_id
CLOUDFLARE_API_TOKEN=your_api_token
CLOUDFLARE_PROJECT_NAME=your_project_name

# Optional
UPDATE_INTERVAL_HOURS=12
STATIC_OUTPUT_DIR=html
SOCKS_PROXY=socks5://host:port
```

The `SOCKS_PROXY` variable routes all scraping traffic through a SOCKS5 proxy, useful when running from cloud IPs that might be blocked. The Cloudflare deployment is not affected.

If no `TMDB_API_KEY` is set, scraping still works -- you just won't get enriched metadata.

## Commands

```
film-finder scrape          Fetch latest showtimes from all sources
film-finder films           List all movies with screening counts
film-finder list            Show all upcoming screenings
film-finder ov              Show only OV/OmU screenings
film-finder search <query>  Search for a movie by title (--ov for OV only)
film-finder static [path]   Generate static website (default: html/)
film-finder deploy          Deploy static site to Cloudflare Pages
film-finder serve           Start service (scrape + static + deploy every N hours)
film-finder stop            Stop the background service
film-finder db-reset        Delete the database and start fresh
```

## Deploying to Cloudflare Pages

1. Create a Cloudflare Pages project:
   - Go to https://dash.cloudflare.com/ > Pages > Create a project > Direct Upload
   - Name it (e.g., `ov-berlin`)
   - Upload any placeholder file to create the project

2. Get your Cloudflare credentials:
   - **Account ID**: Found in the right sidebar of your Cloudflare dashboard
   - **API Token**: Create at https://dash.cloudflare.com/profile/api-tokens
     - Use "Create Custom Token"
     - Permissions: Account > Cloudflare Pages > Edit

3. Add credentials to `.env` and run `film-finder deploy`

Your site will be available at `https://<project-name>.pages.dev`.

## What it scrapes

| Source | Theaters |
|--------|----------|
| UCI Kinowelt | Eastgate, East Side Gallery, Gropius Passagen, Potsdam |
| CineStar | CUBIX Alexanderplatz, Tegel, Hellersdorf, KulturBrauerei |
| Yorck Kinos | 14 theaters (Babylon, Cinema Paris, Delphi, Odeon, Passage, Rollberg, etc.) |
| critic.de | ~50 Berlin cinemas with OV screenings |

**OV** = Original Version (no dubbing)
**OmU** = Original mit Untertiteln (original with German subtitles)
**OmeU** = Original mit englischen Untertiteln (original with English subtitles)

## Makefile

```bash
make help          # Show all targets
make build         # Dev build
make release       # Optimized build
make test          # Run tests
make docker-build  # Build Docker image
make docker-up     # Start container
make docker-scrape # One-shot scrape in container
```

## Adding more cinemas

1. Create `src/scrapers/whatever.rs`
2. Implement the `Scraper` trait (see `uci.rs` for an example)
3. Add it to the scraper list in `main.rs`

The main things to figure out for each site:
- How to get the program page for each theater
- How movie containers are structured in the HTML
- Where showtime links are and how dates/times are encoded
- How OV screenings are marked (usually a CSS class or legend)

## Building

Needs Rust 1.70+ and `cargo build --release`. Compiles to a single binary with SQLite statically linked.
