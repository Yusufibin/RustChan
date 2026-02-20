# RustChan

Un imageboard匿名 (style 4chan) écrit en Rust.

## Stack

- **Backend** : Rust + Axum
- **Base de données** : SQLite (sqlx)
- **Templates** : Tera
- **Async** : Tokio

## Installation

```bash
# Compiler le projet
cargo build

# Lancer le serveur
cargo run

# Avec un port personnalisé
PORT=3001 cargo run
```

## Structure

```
src/
  main.rs      # Point d'entrée, routes
  db/          # Opérations base de données
  handlers/    # Gestionnaires HTTP
  models/      # Structures de données
templates/     # Templates Tera
static/        # CSS, images
uploads/       # Images uploadées
```

## Fonctionnalités

- Création et gestion de boards
- Threads et posts avec/sans images
- Panel d'administration
- Upload d'images
- Système d'authentification admin

## Commandes utiles

```bash
# Vérifier le code sans compiler
cargo check

# Linter
cargo clippy

# Formater le code
cargo fmt
```
