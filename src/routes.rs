use axum::{
    Extension, Json,
    extract::{Multipart, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use maud::html;
use serde::Deserialize;

use crate::{
    AppState, db,
    db::{Cursor, Scope, Visibility},
    ids, images, markdown,
    views::{self, Head, ItemOpts, MAX_POST_CHARS, Nav},
    visibility::Viewer,
};

const PAGE_SIZE: usize = 30;

pub struct AppError(String);

impl<E: std::fmt::Display> From<E> for AppError {
    fn from(e: E) -> Self {
        AppError(e.to_string())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        eprintln!("error: {}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong.").into_response()
    }
}

type Result<T = Response> = std::result::Result<T, AppError>;

pub fn not_found(state: &AppState, viewer: &Viewer) -> Response {
    let page = views::layout(state, viewer, Nav::None, Head::titled("Not found · kvitter"), &[], views::not_found_body());
    (StatusCode::NOT_FOUND, page).into_response()
}

pub async fn fallback(State(state): State<AppState>, Extension(viewer): Extension<Viewer>) -> Response {
    not_found(&state, &viewer)
}

#[derive(Deserialize)]
pub struct Page {
    before: Option<String>,
}

/// One page of posts, plus the "Older posts" link when there are more.
fn page_of(
    state: &AppState,
    viewer: &Viewer,
    scope: &Scope,
    page: &Page,
    path: &str,
) -> Result<(Vec<db::Post>, Option<String>)> {
    let before = page.before.as_deref().and_then(Cursor::decode);
    let mut posts = db::list_posts(&state.db(), viewer, scope, before, PAGE_SIZE + 1)?;
    let older = if posts.len() > PAGE_SIZE {
        posts.truncate(PAGE_SIZE);
        posts.last().map(|p| format!("{path}?before={}", Cursor::of(p).encode()))
    } else {
        None
    };
    Ok((posts, older))
}

fn home_page(state: &AppState, viewer: &Viewer, page: &Page, draft: Option<(&str, Visibility, &str)>) -> Result {
    let (posts, older) = page_of(state, viewer, &Scope::Own, page, "/")?;
    let (body, vis, error) = match draft {
        Some((b, v, e)) => (b, v, Some(e)),
        None => ("", Visibility::Private, None),
    };
    let submit = if vis == Visibility::Public { "Post publicly" } else { "Post privately" };
    let content = html! {
        (views::composer("/posts", body, vis, submit, error))
        (views::posts_list(state, viewer, &posts, &ItemOpts { show_author: false }, older, "Nothing here yet. Your posts are private until you make them public."))
    };
    let status = if error.is_some() { StatusCode::UNPROCESSABLE_ENTITY } else { StatusCode::OK };
    let markup = views::layout(state, viewer, Nav::Mine, Head::titled("kvitter"), &["app.js", "composer.js"], content);
    Ok((status, markup).into_response())
}

pub async fn home(State(state): State<AppState>, Extension(viewer): Extension<Viewer>, Query(page): Query<Page>) -> Result {
    home_page(&state, &viewer, &page, None)
}

pub async fn explore(State(state): State<AppState>, Extension(viewer): Extension<Viewer>, Query(page): Query<Page>) -> Result {
    let (posts, older) = page_of(&state, &viewer, &Scope::Public, &page, "/explore")?;
    let content = html! {
        (views::page_head(html! { "Explore" }, Some("Public posts from everyone".into())))
        (views::posts_list(&state, &viewer, &posts, &ItemOpts { show_author: true }, older, "No public posts yet."))
    };
    let scripts: &[&str] = if viewer.is_signed_in() { &["app.js"] } else { &[] };
    Ok(views::layout(&state, &viewer, Nav::Explore, Head::titled("Explore · kvitter"), scripts, content).into_response())
}

#[derive(Deserialize)]
pub struct PostForm {
    body: String,
    #[serde(default)]
    visibility: String,
}

impl PostForm {
    fn visibility(&self) -> Visibility {
        // Anything unexpected falls back to private.
        Visibility::parse(&self.visibility).unwrap_or(Visibility::Private)
    }

    fn validate(&self) -> std::result::Result<&str, &'static str> {
        let body = self.body.trim();
        if body.is_empty() {
            Err("Write something first.")
        } else if body.chars().count() > MAX_POST_CHARS {
            Err("Posts can be at most 100,000 characters.")
        } else {
            Ok(body)
        }
    }
}

pub async fn create(State(state): State<AppState>, Extension(viewer): Extension<Viewer>, axum::Form(form): axum::Form<PostForm>) -> Result {
    let Some(author) = viewer.user() else { return Ok(not_found(&state, &viewer)) };
    let body = match form.validate() {
        Ok(b) => b,
        Err(e) => return home_page(&state, &viewer, &Page { before: None }, Some((&form.body, form.visibility(), e))),
    };
    db::create_post(&mut state.db(), author, body, form.visibility(), &markdown::analyze(body))?;
    Ok(Redirect::to("/").into_response())
}

pub async fn post_page(State(state): State<AppState>, Extension(viewer): Extension<Viewer>, Path(id): Path<String>) -> Result {
    let post = if ids::is_valid(&id) { db::get_post(&state.db(), &viewer, &id)? } else { None };
    let Some(post) = post else { return Ok(not_found(&state, &viewer)) };
    let head = Head {
        title: format!("{}: {} · kvitter", post.author, markdown::excerpt(&post.body, 60)),
        meta: views::public_post_meta(&state, &post),
    };
    let scripts: &[&str] = if viewer.owns(&post.author) { &["app.js"] } else { &[] };
    Ok(views::layout(&state, &viewer, Nav::None, head, scripts, views::single_post(&state, &viewer, &post)).into_response())
}

/// The viewer's own post, or None (shown as 404 either way).
fn own_post(state: &AppState, viewer: &Viewer, id: &str) -> Result<Option<db::Post>> {
    if !ids::is_valid(id) {
        return Ok(None);
    }
    Ok(db::get_post(&state.db(), viewer, id)?.filter(|p| viewer.owns(&p.author)))
}

fn edit_response(state: &AppState, viewer: &Viewer, post: &db::Post, body: &str, vis: Visibility, error: Option<&str>) -> Response {
    let status = if error.is_some() { StatusCode::UNPROCESSABLE_ENTITY } else { StatusCode::OK };
    let page = views::layout(
        state,
        viewer,
        Nav::None,
        Head::titled("Edit · kvitter"),
        &["app.js", "composer.js"],
        views::edit_page(post, body, vis, error),
    );
    (status, page).into_response()
}

pub async fn edit_page(State(state): State<AppState>, Extension(viewer): Extension<Viewer>, Path(id): Path<String>) -> Result {
    let Some(post) = own_post(&state, &viewer, &id)? else { return Ok(not_found(&state, &viewer)) };
    Ok(edit_response(&state, &viewer, &post, &post.body, post.visibility, None))
}

pub async fn edit(
    State(state): State<AppState>,
    Extension(viewer): Extension<Viewer>,
    Path(id): Path<String>,
    axum::Form(form): axum::Form<PostForm>,
) -> Result {
    let Some(post) = own_post(&state, &viewer, &id)? else { return Ok(not_found(&state, &viewer)) };
    let body = match form.validate() {
        Ok(b) => b,
        Err(e) => return Ok(edit_response(&state, &viewer, &post, &form.body, form.visibility(), Some(e))),
    };
    db::update_post(&mut state.db(), &viewer, &id, body, form.visibility(), &markdown::analyze(body))?;
    Ok(Redirect::to(&format!("/p/{id}")).into_response())
}

#[derive(Deserialize)]
pub struct VisibilityForm {
    visibility: String,
}

pub async fn set_visibility(
    State(state): State<AppState>,
    Extension(viewer): Extension<Viewer>,
    Path(id): Path<String>,
    axum::Form(form): axum::Form<VisibilityForm>,
) -> Result {
    let vis = Visibility::parse(&form.visibility).unwrap_or(Visibility::Private);
    if !ids::is_valid(&id) || !db::set_visibility(&state.db(), &viewer, &id, vis)? {
        return Ok(not_found(&state, &viewer));
    }
    Ok(Redirect::to(&format!("/p/{id}")).into_response())
}

pub async fn delete(State(state): State<AppState>, Extension(viewer): Extension<Viewer>, Path(id): Path<String>) -> Result {
    let removed = if ids::is_valid(&id) { db::delete_post(&mut state.db(), &viewer, &id)? } else { None };
    let Some(removed) = removed else { return Ok(not_found(&state, &viewer)) };
    for name in removed {
        remove_upload(&state, Some(name));
    }
    Ok(Redirect::to("/").into_response())
}

pub async fn user_page(
    State(state): State<AppState>,
    Extension(viewer): Extension<Viewer>,
    Path(name): Path<String>,
    Query(page): Query<Page>,
) -> Result {
    let scope = Scope::Author(&name);
    let counts = db::count_posts(&state.db(), &viewer, &scope)?;
    let own = viewer.owns(&name);
    // A user with nothing visible to this viewer doesn't exist, as far as they can tell.
    if counts.total == 0 && !own {
        return Ok(not_found(&state, &viewer));
    }
    let path = format!("/u/{}", markdown::encode_path(&name));
    let (posts, older) = page_of(&state, &viewer, &scope, &page, &path)?;
    let sub = if own {
        format!("{} · {} public", plural(counts.total, "post"), counts.public)
    } else {
        format!("{} public", plural(counts.total, "post"))
    };
    let avatar = db::avatar_of(&state.db(), &name)?;
    let content = html! {
        (views::user_head(&name, avatar.as_deref(), sub, own))
        (views::posts_list(&state, &viewer, &posts, &ItemOpts { show_author: false }, older, "No posts yet."))
    };
    Ok(views::layout(&state, &viewer, Nav::None, Head::titled(format!("{name} · kvitter")), &["app.js"], content).into_response())
}

pub async fn tag_page(
    State(state): State<AppState>,
    Extension(viewer): Extension<Viewer>,
    Path(tag): Path<String>,
    Query(page): Query<Page>,
) -> Result {
    let tag = tag.to_lowercase();
    let scope = Scope::Tag(&tag);
    let counts = db::count_posts(&state.db(), &viewer, &scope)?;
    if counts.total == 0 {
        return Ok(not_found(&state, &viewer));
    }
    let path = format!("/t/{}", markdown::encode_path(&tag));
    let (posts, older) = page_of(&state, &viewer, &scope, &page, &path)?;
    let content = html! {
        (views::page_head(html! { span.hash { "#" } (tag) }, Some(plural(counts.total, "post"))))
        (views::posts_list(&state, &viewer, &posts, &ItemOpts { show_author: true }, older, ""))
    };
    Ok(views::layout(&state, &viewer, Nav::None, Head::titled(format!("#{tag} · kvitter")), &["app.js"], content).into_response())
}

fn plural(n: i64, word: &str) -> String {
    if n == 1 { format!("1 {word}") } else { format!("{n} {word}s") }
}

#[derive(Deserialize)]
pub struct TagQuery {
    #[serde(default)]
    q: String,
}

pub async fn api_tags(State(state): State<AppState>, Extension(viewer): Extension<Viewer>, Query(q): Query<TagQuery>) -> Result {
    let tags = db::tags_with_prefix(&state.db(), &viewer, q.q.trim_start_matches('#'), 20)?;
    let json: Vec<_> = tags.into_iter().map(|(tag, count)| serde_json::json!({ "tag": tag, "count": count })).collect();
    Ok(Json(json).into_response())
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

pub async fn upload(State(state): State<AppState>, Extension(viewer): Extension<Viewer>, mut multipart: Multipart) -> Result {
    let Some(owner) = viewer.user() else { return Ok(json_error(StatusCode::NOT_FOUND, "Not found")) };
    let mut data = None;
    while let Some(field) = multipart.next_field().await.map_err(|e| e.body_text())? {
        if field.name() == Some("file") {
            data = Some(field.bytes().await.map_err(|e| e.body_text())?);
        }
    }
    let Some(data) = data else { return Ok(json_error(StatusCode::BAD_REQUEST, "No file in the upload.")) };
    let processed = match tokio::task::spawn_blocking(move || images::process(&data)).await? {
        Ok(p) => p,
        Err(e) => return Ok(json_error(StatusCode::UNSUPPORTED_MEDIA_TYPE, &e)),
    };
    let id = db::create_file(&state.db(), owner, processed.ext, processed.content_type)?;
    let path = state.config.uploads_dir.join(format!("{id}.{}", processed.ext));
    tokio::task::spawn_blocking(move || std::fs::write(path, processed.bytes)).await??;
    Ok(Json(serde_json::json!({
        "id": id,
        "url": format!("/files/{id}"),
        "markdown": format!("![](/files/{id})"),
    }))
    .into_response())
}

fn settings(state: &AppState, viewer: &Viewer, user: &str, error: Option<&str>) -> Result {
    let avatar = db::avatar_of(&state.db(), user)?;
    let content = views::settings_page(user, avatar.as_deref(), error);
    let page = views::layout(state, viewer, Nav::None, Head::titled("Profile picture · kvitter"), &[], content);
    let status = if error.is_some() { StatusCode::UNPROCESSABLE_ENTITY } else { StatusCode::OK };
    Ok((status, page).into_response())
}

pub async fn settings_page(State(state): State<AppState>, Extension(viewer): Extension<Viewer>) -> Result {
    let Some(user) = viewer.user() else { return Ok(not_found(&state, &viewer)) };
    settings(&state, &viewer, user, None)
}

pub async fn set_avatar(State(state): State<AppState>, Extension(viewer): Extension<Viewer>, mut multipart: Multipart) -> Result {
    let Some(user) = viewer.user() else { return Ok(not_found(&state, &viewer)) };
    let mut data = None;
    while let Some(field) = multipart.next_field().await.map_err(|e| e.body_text())? {
        if field.name() == Some("file") {
            data = Some(field.bytes().await.map_err(|e| e.body_text())?);
        }
    }
    let Some(data) = data.filter(|d| !d.is_empty()) else { return settings(&state, &viewer, user, Some("Choose a picture first.")) };
    let processed = match tokio::task::spawn_blocking(move || images::avatar(&data)).await? {
        Ok(p) => p,
        Err(e) => return settings(&state, &viewer, user, Some(&e)),
    };
    let id = db::create_file(&state.db(), user, processed.ext, processed.content_type)?;
    let path = state.config.uploads_dir.join(format!("{id}.{}", processed.ext));
    tokio::task::spawn_blocking(move || std::fs::write(path, processed.bytes)).await??;
    let old = db::set_avatar(&mut state.db(), user, Some(&id))?;
    remove_upload(&state, old);
    Ok(Redirect::to("/settings").into_response())
}

pub async fn remove_avatar(State(state): State<AppState>, Extension(viewer): Extension<Viewer>) -> Result {
    let Some(user) = viewer.user() else { return Ok(not_found(&state, &viewer)) };
    let old = db::set_avatar(&mut state.db(), user, None)?;
    remove_upload(&state, old);
    Ok(Redirect::to("/settings").into_response())
}

fn remove_upload(state: &AppState, name: Option<String>) {
    if let Some(name) = name {
        let path = state.config.uploads_dir.join(&name);
        if let Err(e) = std::fs::remove_file(&path) {
            eprintln!("couldn't remove {}: {e}", path.display());
        }
    }
}

pub async fn serve_file(
    State(state): State<AppState>,
    Extension(viewer): Extension<Viewer>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result {
    let file = if ids::is_valid(&id) { db::get_file(&state.db(), &viewer, &id)? } else { None };
    let Some(file) = file else { return Ok(not_found(&state, &viewer)) };
    // Stored files never change, so the id is the ETag. Visibility can change, so anonymous
    // caches must revalidate every time.
    let etag = format!("\"{}\"", file.public_id);
    let cache = HeaderValue::from_static("public, no-cache");
    if headers.get(header::IF_NONE_MATCH).is_some_and(|v| v.as_bytes() == etag.as_bytes()) {
        return Ok((StatusCode::NOT_MODIFIED, [(header::ETAG, etag)], [(header::CACHE_CONTROL, cache)]).into_response());
    }
    let path = state.config.uploads_dir.join(&file.name);
    let bytes = match tokio::task::spawn_blocking(move || std::fs::read(path)).await? {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(not_found(&state, &viewer)),
        Err(e) => return Err(e.into()),
    };
    Ok((
        [(header::CONTENT_TYPE, file.content_type), (header::ETAG, etag)],
        [(header::CACHE_CONTROL, cache)],
        bytes,
    )
        .into_response())
}
