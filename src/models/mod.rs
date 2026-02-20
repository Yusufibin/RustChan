use serde::{Deserialize, Serialize};
use chrono::NaiveDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Board {
    pub id: i64,
    pub slug: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Post {
    pub id: i64,
    pub board_slug: String,
    pub thread_id: Option<i64>,
    pub author_id: String,
    pub content: String,
    pub image_path: Option<String>,
    pub image_name: Option<String>,
    pub created_at: NaiveDateTime,
    pub bump_time: NaiveDateTime,
}

#[allow(dead_code)]
impl Post {
    pub fn is_thread(&self) -> bool {
        self.thread_id.is_none()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewPost {
    pub content: String,
    pub image_data: Option<String>,
    pub image_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub op: Post,
    pub replies: Vec<Post>,
    pub reply_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewBoard {
    pub slug: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AdminStats {
    pub total_boards: i64,
    pub total_posts: i64,
    pub total_threads: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct PostWithBoard {
    pub id: i64,
    pub board_slug: String,
    pub thread_id: Option<i64>,
    pub author_id: String,
    pub content: String,
    pub image_path: Option<String>,
    pub image_name: Option<String>,
    pub created_at: NaiveDateTime,
    pub bump_time: NaiveDateTime,
}
