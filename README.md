
# RustChan

An anonymous imageboard (4chan style) written in Rust.

![screenshot](https://raw.githubusercontent.com/Yusufibin/RustChan/refs/heads/main/Capture%20d%E2%80%99%C3%A9cran_2026-04-15_07-29-17.png)

## Stack

- **Backend**: Rust + Axum
- **Database**: SQLite (sqlx)
- **Templates**: Tera
- **Async**: Tokio

## Installation

```bash
# Build the project
cargo build

# Run the server
cargo run

# With a custom port
PORT=3001 cargo run
```

## Structure

```
src/
  main.rs      # Entry point, routes
  db/          # Database operations
  handlers/    # HTTP handlers
  models/      # Data structures
templates/     # Tera templates
static/        # CSS, images
uploads/       # Uploaded images
```

## Features

- Board creation and management
- Threads and posts with/without images
- Administration panel
- Image uploading
- Admin authentication system

## Useful commands

```bash
# Check the code without compiling
cargo check

# Linter
cargo clippy

# Format the code
cargo fmt
```

![screenshot](https://raw.githubusercontent.com/Yusufibin/RustChan/refs/heads/main/Capture%20d%E2%80%99%C3%A9cran_2026-04-15_07-29-45.png)

![screenshot](https://raw.githubusercontent.com/Yusufibin/RustChan/refs/heads/main/Capture%20d%E2%80%99%C3%A9cran_2026-04-15_07-30-19.png)
