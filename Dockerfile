# Stage 1: Build the Rust binary
FROM rust:1.86-slim AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release --locked

# Stage 2: Minimal runtime image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install wrangler globally for Cloudflare Pages deployment
RUN npm install -g wrangler

COPY --from=builder /build/target/release/film-finder /usr/local/bin/film-finder

WORKDIR /data

# Default: run the serve loop in foreground (scrape -> static -> deploy every N hours)
# Override with: docker run ... film-finder scrape  (one-shot)
ENTRYPOINT ["film-finder"]
CMD ["serve"]
