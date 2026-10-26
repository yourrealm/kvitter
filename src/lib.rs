pub mod auth;
pub mod config;
pub mod db;
pub mod feeds;
pub mod ids;
pub mod images;
pub mod import;
pub mod markdown;
pub mod routes;
pub mod statics;
pub mod views;
pub mod visibility;

use std::sync::{Arc, Mutex, MutexGuard};

use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{get, post},
};
use rusqlite::Connection;

use crate::config::Config;

/// Matches `maxRequestSizeMb` on the Realm router.
pub const UPLOAD_LIMIT: usize = 25 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    db: Arc<Mutex<Connection>>,
}

impl AppState {
    pub fn new(config: Config, conn: Connection) -> Self {
        AppState { config: Arc::new(config), db: Arc::new(Mutex::new(conn)) }
    }

    /// Never hold this across an `.await`.
    pub fn db(&self) -> MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(routes::home))
        .route("/explore", get(routes::explore))
        .route("/posts", post(routes::create))
        .route("/p/{id}", get(routes::post_page))
        .route("/p/{id}/edit", get(routes::edit_page).post(routes::edit))
        .route("/p/{id}/visibility", post(routes::set_visibility))
        .route("/p/{id}/delete", post(routes::delete))
        .route("/u/{name}", get(routes::user_page))
        .route("/t/{tag}", get(routes::tag_page))
        .route("/api/tags", get(routes::api_tags))
        .route("/files", post(routes::upload).layer(DefaultBodyLimit::max(UPLOAD_LIMIT)))
        .route("/files/{id}", get(routes::serve_file))
        .route("/settings", get(routes::settings_page))
        .route("/settings/avatar", post(routes::set_avatar).layer(DefaultBodyLimit::max(UPLOAD_LIMIT)))
        .route("/settings/avatar/delete", post(routes::remove_avatar))
        .route("/static/{name}", get(statics::serve))
        .route("/rss.xml", get(feeds::rss))
        .route("/sitemap.xml", get(feeds::sitemap))
        .route("/robots.txt", get(feeds::robots))
        .route("/healthz", get(|| async { "ok" }))
        .fallback(routes::fallback)
        .layer(middleware::from_fn_with_state(state.clone(), auth::identify))
        .with_state(state)
}
