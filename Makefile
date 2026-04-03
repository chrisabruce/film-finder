# Film Finder Makefile
# Build and deployment targets for the film-finder application

BINARY_NAME := film-finder
INSTALL_DIR := /usr/local/bin
SERVICE_NAME := film-finder
SERVICE_FILE := /etc/systemd/system/$(SERVICE_NAME).service
CONFIG_DIR := /etc/film-finder

# SVC_USER must be passed explicitly (e.g. make install-service SVC_USER=myuser)
# We use SVC_USER instead of USER to avoid conflict with the shell environment variable.
SVC_USER ?=

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

# Install the binary to system (requires sudo)
.PHONY: install
install: release
	sudo install -Dm755 target/release/$(BINARY_NAME) $(INSTALL_DIR)/$(BINARY_NAME)
	@echo "Installed $(BINARY_NAME) to $(INSTALL_DIR)"

# Install production binary (requires sudo)
.PHONY: install-production
install-production: production
	sudo install -Dm755 target/release/$(BINARY_NAME) $(INSTALL_DIR)/$(BINARY_NAME)
	@echo "Installed $(BINARY_NAME) (production build) to $(INSTALL_DIR)"

# Shared helper for creating the systemd service (called by install-service targets)
define install-service-impl
	@if [ -z "$(SVC_USER)" ]; then \
		echo "Error: SVC_USER variable required. Usage: make $@ SVC_USER=myuser"; \
		exit 1; \
	fi
	@echo "Creating systemd service for user $(SVC_USER)..."
	sudo mkdir -p $(CONFIG_DIR)
	@if [ -f .env ]; then \
		sudo cp .env $(CONFIG_DIR)/.env; \
		sudo chmod 600 $(CONFIG_DIR)/.env; \
		echo "Copied .env to $(CONFIG_DIR)/.env"; \
	fi
	@printf '%s\n' \
		'[Unit]' \
		'Description=Film Finder - Movie showtime aggregator' \
		'After=network.target' \
		'' \
		'[Service]' \
		'Type=simple' \
		'User=$(SVC_USER)' \
		'Group=$(SVC_USER)' \
		'WorkingDirectory=$(CONFIG_DIR)' \
		'EnvironmentFile=-$(CONFIG_DIR)/.env' \
		'ExecStart=$(INSTALL_DIR)/$(BINARY_NAME) serve --daemon' \
		'Restart=always' \
		'RestartSec=10' \
		'' \
		'# Security hardening' \
		'NoNewPrivileges=true' \
		'ProtectSystem=strict' \
		'ProtectHome=read-only' \
		'PrivateTmp=true' \
		'ReadWritePaths=$(CONFIG_DIR)' \
		'' \
		'[Install]' \
		'WantedBy=multi-user.target' \
		| sudo tee $(SERVICE_FILE) > /dev/null
	sudo systemctl daemon-reload
	sudo systemctl enable $(SERVICE_NAME)
	@echo ""
	@echo "Service installed and enabled!"
	@echo "Commands:"
	@echo "  sudo systemctl start $(SERVICE_NAME)    - Start the service"
	@echo "  sudo systemctl stop $(SERVICE_NAME)     - Stop the service"
	@echo "  sudo systemctl status $(SERVICE_NAME)   - Check status"
	@echo "  sudo journalctl -u $(SERVICE_NAME) -f   - View logs"
endef

# Create systemd service file (requires sudo)
# Usage: make install-service SVC_USER=myuser
.PHONY: install-service
install-service: install
	$(install-service-impl)

# Install production service
# Usage: make install-service-production SVC_USER=myuser
.PHONY: install-service-production
install-service-production: install-production
	$(install-service-impl)

# Start the service
.PHONY: start
start:
	sudo systemctl start $(SERVICE_NAME)

# Stop the service
.PHONY: stop
stop:
	sudo systemctl stop $(SERVICE_NAME)

# Restart the service
.PHONY: restart
restart:
	sudo systemctl restart $(SERVICE_NAME)

# Check service status
.PHONY: status
status:
	sudo systemctl status $(SERVICE_NAME)

# View service logs
.PHONY: logs
logs:
	sudo journalctl -u $(SERVICE_NAME) -f

# Uninstall service and binary
.PHONY: uninstall
uninstall:
	-sudo systemctl stop $(SERVICE_NAME)
	-sudo systemctl disable $(SERVICE_NAME)
	-sudo rm -f $(SERVICE_FILE)
	-sudo rm -f $(INSTALL_DIR)/$(BINARY_NAME)
	-sudo rm -rf $(CONFIG_DIR)
	sudo systemctl daemon-reload
	@echo "Service and binary uninstalled"

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
	@echo "Installation targets (require sudo):"
	@echo "  make install            - Install release binary to $(INSTALL_DIR)"
	@echo "  make install-production - Install production binary to $(INSTALL_DIR)"
	@echo "  make uninstall          - Remove service and binary"
	@echo ""
	@echo "Service targets (Pop!_OS/systemd, require sudo):"
	@echo "  make install-service SVC_USER=myuser            - Install as systemd service (release)"
	@echo "  make install-service-production SVC_USER=myuser - Install as systemd service (production)"
	@echo "  make start              - Start the service"
	@echo "  make stop               - Stop the service"
	@echo "  make restart            - Restart the service"
	@echo "  make status             - Check service status"
	@echo "  make logs               - View service logs (follow mode)"
