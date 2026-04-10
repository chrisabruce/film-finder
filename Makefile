# Film Finder Makefile
# Build and deployment targets for the film-finder application

BINARY_NAME := film-finder

# Default target
.PHONY: all
all: build

# Development build
.PHONY: build
build:
	cargo build

# Release build (optimized)
.PHONY: release
release:
	cargo build --release

# Production build (release with additional optimizations)
.PHONY: production
production:
	RUSTFLAGS="-C target-cpu=native" cargo build --release --locked

# Run in development mode
.PHONY: run
run:
	cargo run

# Run in release mode
.PHONY: run-release
run-release:
	cargo run --release

# Run tests
.PHONY: test
test:
	cargo test

# Clean build artifacts
.PHONY: clean
clean:
	cargo clean

# Docker targets
.PHONY: docker-build
docker-build:
	docker build -t $(BINARY_NAME) .

.PHONY: docker-up
docker-up:
	docker compose up -d

.PHONY: docker-down
docker-down:
	docker compose down

.PHONY: docker-logs
docker-logs:
	docker compose logs -f

.PHONY: docker-scrape
docker-scrape:
	docker compose run --rm $(BINARY_NAME) scrape

# Help
.PHONY: help
help:
	@echo "Film Finder Makefile Targets:"
	@echo ""
	@echo "Build targets:"
	@echo "  make build              - Development build"
	@echo "  make release            - Release build (optimized)"
	@echo "  make production         - Production build (release + native CPU optimizations)"
	@echo "  make clean              - Clean build artifacts"
	@echo ""
	@echo "Run targets:"
	@echo "  make run                - Run in development mode"
	@echo "  make run-release        - Run in release mode"
	@echo "  make test               - Run tests"
	@echo ""
	@echo "Docker targets:"
	@echo "  make docker-build       - Build Docker image"
	@echo "  make docker-up          - Start container (serve loop)"
	@echo "  make docker-down        - Stop container"
	@echo "  make docker-logs        - View container logs"
	@echo "  make docker-scrape      - One-shot scrape in container"
