use anyhow::Result;
use sqlx::SqlitePool;
use crate::models::{AdminStats, Board, Post, PostWithBoard, Thread};

pub async fn get_boards(pool: &SqlitePool) -> Result<Vec<Board>> {
    let boards = sqlx::query_as::<_, Board>("SELECT id, slug, name, description FROM boards ORDER BY slug")
        .fetch_all(pool)
        .await?;
    Ok(boards)
}

pub async fn get_board(pool: &SqlitePool, slug: &str) -> Result<Option<Board>> {
    let board = sqlx::query_as::<_, Board>("SELECT id, slug, name, description FROM boards WHERE slug = ?")
        .bind(slug)
        .fetch_optional(pool)
        .await?;
    Ok(board)
}

pub async fn get_threads(pool: &SqlitePool, board_slug: &str) -> Result<Vec<Thread>> {
    let ops = sqlx::query_as::<_, Post>(
        "SELECT id, board_slug, thread_id, author_id, content, image_path, image_name, created_at, bump_time 
         FROM posts WHERE board_slug = ? AND thread_id IS NULL 
         ORDER BY bump_time DESC LIMIT 10"
    )
    .bind(board_slug)
    .fetch_all(pool)
    .await?;

    let mut threads = Vec::new();
    for op in ops {
        let replies = sqlx::query_as::<_, Post>(
            "SELECT id, board_slug, thread_id, author_id, content, image_path, image_name, created_at, bump_time 
             FROM posts WHERE thread_id = ? 
             ORDER BY created_at DESC LIMIT 3"
        )
        .bind(op.id)
        .fetch_all(pool)
        .await?;

        let reply_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE thread_id = ?")
            .bind(op.id)
            .fetch_one(pool)
            .await?;

        threads.push(Thread {
            replies: replies.into_iter().rev().collect(),
            reply_count,
            op,
        });
    }
    Ok(threads)
}

pub async fn get_thread(pool: &SqlitePool, thread_id: i64) -> Result<Option<Thread>> {
    let op = sqlx::query_as::<_, Post>(
        "SELECT id, board_slug, thread_id, author_id, content, image_path, image_name, created_at, bump_time 
         FROM posts WHERE id = ? AND thread_id IS NULL"
    )
    .bind(thread_id)
    .fetch_optional(pool)
    .await?;

    match op {
        Some(op) => {
            let replies = sqlx::query_as::<_, Post>(
                "SELECT id, board_slug, thread_id, author_id, content, image_path, image_name, created_at, bump_time 
                 FROM posts WHERE thread_id = ? 
                 ORDER BY created_at ASC"
            )
            .bind(thread_id)
            .fetch_all(pool)
            .await?;

            let reply_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE thread_id = ?")
                .bind(thread_id)
                .fetch_one(pool)
                .await?;

            Ok(Some(Thread { op, replies, reply_count }))
        }
        None => Ok(None),
    }
}

pub async fn create_post(
    pool: &SqlitePool,
    board_slug: &str,
    thread_id: Option<i64>,
    author_id: &str,
    content: &str,
    image_path: Option<&str>,
    image_name: Option<&str>,
) -> Result<i64> {
    let now = chrono::Utc::now().naive_utc();
    
    let result = sqlx::query(
        "INSERT INTO posts (board_slug, thread_id, author_id, content, image_path, image_name, created_at, bump_time) 
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(board_slug)
    .bind(thread_id)
    .bind(author_id)
    .bind(content)
    .bind(image_path)
    .bind(image_name)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    let post_id = result.last_insert_rowid();

    if thread_id.is_some() {
        sqlx::query("UPDATE posts SET bump_time = ? WHERE id = ?")
            .bind(now)
            .bind(thread_id)
            .execute(pool)
            .await?;
    }

    Ok(post_id)
}

pub async fn init_db(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS boards (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            slug TEXT UNIQUE NOT NULL,
            name TEXT NOT NULL,
            description TEXT NOT NULL
        )"
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS posts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            board_slug TEXT NOT NULL,
            thread_id INTEGER,
            author_id TEXT NOT NULL,
            content TEXT NOT NULL,
            image_path TEXT,
            image_name TEXT,
            created_at DATETIME NOT NULL,
            bump_time DATETIME NOT NULL,
            FOREIGN KEY (board_slug) REFERENCES boards(slug),
            FOREIGN KEY (thread_id) REFERENCES posts(id)
        )"
    )
    .execute(pool)
    .await?;

    let boards = [
        ("b", "Random", "The stories and information posted here are artistic works of fiction and falsehood."),
        ("g", "Technology", "Install Gentoo"),
        ("v", "Video Games", "vidya gaems"),
        ("a", "Anime & Manga", "Discuss Chinese cartoons"),
        ("pol", "Politics", "Political discussion"),
    ];

    for (slug, name, desc) in boards {
        sqlx::query("INSERT OR IGNORE INTO boards (slug, name, description) VALUES (?, ?, ?)")
            .bind(slug)
            .bind(name)
            .bind(desc)
            .execute(pool)
            .await?;
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS admin_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            token TEXT UNIQUE NOT NULL,
            created_at DATETIME NOT NULL
        )"
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn create_board(pool: &SqlitePool, slug: &str, name: &str, description: &str) -> Result<()> {
    sqlx::query("INSERT INTO boards (slug, name, description) VALUES (?, ?, ?)")
        .bind(slug)
        .bind(name)
        .bind(description)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_board(pool: &SqlitePool, slug: &str, name: &str, description: &str) -> Result<()> {
    sqlx::query("UPDATE boards SET name = ?, description = ? WHERE slug = ?")
        .bind(name)
        .bind(description)
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_board(pool: &SqlitePool, slug: &str) -> Result<()> {
    sqlx::query("DELETE FROM posts WHERE board_slug = ?")
        .bind(slug)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM boards WHERE slug = ?")
        .bind(slug)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_admin_stats(pool: &SqlitePool) -> Result<AdminStats> {
    let total_boards: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM boards")
        .fetch_one(pool)
        .await?;

    let total_posts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts")
        .fetch_one(pool)
        .await?;

    let total_threads: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts WHERE thread_id IS NULL")
        .fetch_one(pool)
        .await?;

    Ok(AdminStats {
        total_boards,
        total_posts,
        total_threads,
    })
}

pub async fn get_all_posts(pool: &SqlitePool, limit: i64, offset: i64) -> Result<Vec<PostWithBoard>> {
    let posts = sqlx::query_as::<_, PostWithBoard>(
        "SELECT id, board_slug, thread_id, author_id, content, image_path, image_name, created_at, bump_time 
         FROM posts ORDER BY created_at DESC LIMIT ? OFFSET ?"
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(posts)
}

pub async fn get_post_count(pool: &SqlitePool) -> Result<i64> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM posts")
        .fetch_one(pool)
        .await?;
    Ok(count)
}

pub async fn delete_post(pool: &SqlitePool, post_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM posts WHERE id = ? OR thread_id = ?")
        .bind(post_id)
        .bind(post_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn lock_thread(pool: &SqlitePool, thread_id: i64) -> Result<()> {
    sqlx::query("UPDATE posts SET content = CONCAT('[LOCKED] ', content) WHERE id = ?")
        .bind(thread_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn move_thread(pool: &SqlitePool, thread_id: i64, new_board_slug: &str) -> Result<()> {
    let op: Option<Post> = sqlx::query_as(
        "SELECT id, board_slug, thread_id, author_id, content, image_path, image_name, created_at, bump_time 
         FROM posts WHERE id = ? AND thread_id IS NULL"
    )
    .bind(thread_id)
    .fetch_optional(pool)
    .await?;

    if let Some(_op) = op {
        sqlx::query("UPDATE posts SET board_slug = ? WHERE id = ? OR thread_id = ?")
            .bind(new_board_slug)
            .bind(thread_id)
            .bind(thread_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}
