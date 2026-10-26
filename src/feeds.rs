//! RSS, sitemap and robots.txt. These only ever list public posts, whoever asks, and their
//! links use `PUBLIC_URL`. Without a public domain there is nothing to list.

use axum::{
    extract::State,
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::DateTime;

use crate::{
    AppState, db,
    db::Scope,
    markdown::{self, RenderOpts, escape},
    routes::AppError,
    visibility::Viewer,
};

pub async fn rss(State(state): State<AppState>) -> Result<Response, AppError> {
    let base = &state.config.public_url;
    if base.is_empty() {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let posts = db::list_posts(&state.db(), &Viewer::Anonymous, &Scope::Public, None, 50)?;
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n<channel>\n\
         <title>kvitter</title>\n<link>{b}/explore</link>\n<description>Public posts</description>\n\
         <atom:link rel=\"self\" type=\"application/rss+xml\" href=\"{b}/rss.xml\"/>\n",
        b = escape(base)
    );
    for p in &posts {
        let url = format!("{base}/p/{}", p.public_id);
        let html = markdown::render(&p.body, &RenderOpts { base, link_tags: false });
        let date = DateTime::from_timestamp(p.created_at, 0).unwrap_or_default().to_rfc2822();
        xml.push_str(&format!(
            "<item>\n<title>{}</title>\n<link>{u}</link>\n<guid isPermaLink=\"true\">{u}</guid>\n\
             <pubDate>{date}</pubDate>\n<author>{}</author>\n<description>{}</description>\n",
            escape(&markdown::excerpt(&p.body, 80)),
            escape(&p.author),
            escape(&html),
            u = escape(&url),
        ));
        for tag in markdown::analyze(&p.body).tags {
            xml.push_str(&format!("<category>{}</category>\n", escape(&tag)));
        }
        xml.push_str("</item>\n");
    }
    xml.push_str("</channel>\n</rss>\n");
    Ok(([(header::CONTENT_TYPE, "application/rss+xml; charset=utf-8")], xml).into_response())
}

pub async fn sitemap(State(state): State<AppState>) -> Result<Response, AppError> {
    let base = &state.config.public_url;
    if base.is_empty() {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    let posts = db::list_posts(&state.db(), &Viewer::Anonymous, &Scope::Public, None, 50_000)?;
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n<url><loc>{}/explore</loc></url>\n",
        escape(base)
    );
    for p in &posts {
        let lastmod = DateTime::from_timestamp(p.updated_at, 0).unwrap_or_default().format("%Y-%m-%d");
        xml.push_str(&format!(
            "<url><loc>{}/p/{}</loc><lastmod>{lastmod}</lastmod></url>\n",
            escape(base),
            p.public_id
        ));
    }
    xml.push_str("</urlset>\n");
    Ok(([(header::CONTENT_TYPE, "application/xml; charset=utf-8")], xml).into_response())
}

pub async fn robots(State(state): State<AppState>) -> Response {
    let base = &state.config.public_url;
    let body = if base.is_empty() {
        "User-agent: *\nDisallow: /\n".to_string()
    } else {
        format!("User-agent: *\nAllow: /\nSitemap: {base}/sitemap.xml\n")
    };
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}
