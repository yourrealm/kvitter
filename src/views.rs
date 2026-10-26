//! HTML pages, in the "Notebook" direction: serif posts, sans chrome, mono timestamps,
//! hairline rules and one ink-blue accent that also marks what is public.

use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use maud::{DOCTYPE, Markup, PreEscaped, html};

use crate::{
    AppState, auth,
    db::{Post, Visibility},
    markdown::{self, RenderOpts, encode_path},
    statics,
    visibility::Viewer,
};

pub const MAX_POST_CHARS: usize = 100_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Mine,
    Explore,
    None,
}

pub struct Head {
    pub title: String,
    /// Extra tags for public posts: canonical, og:*.
    pub meta: Option<Markup>,
}

impl Head {
    pub fn titled(title: impl Into<String>) -> Self {
        Head { title: title.into(), meta: None }
    }
}

pub fn layout(state: &AppState, viewer: &Viewer, nav: Nav, head: Head, scripts: &[&str], content: Markup) -> Markup {
    let public_url = &state.config.public_url;
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="color-scheme" content="light dark";
                title { (head.title) }
                link rel="stylesheet" href=(statics::url("app.css"));
                link rel="icon" href=(statics::url("favicon.svg")) type="image/svg+xml";
                @if !viewer.is_signed_in() && !public_url.is_empty() {
                    link rel="alternate" type="application/rss+xml" title="kvitter" href={ (public_url) "/rss.xml" };
                }
                @if let Some(meta) = head.meta { (meta) }
                @if scripts.contains(&"composer.js") {
                    meta name="csp-nonce" content=(auth::nonce());
                }
                @for script in scripts {
                    script src=(statics::url(script)) defer {}
                }
            }
            body {
                (PreEscaped(ICONS))
                div.app {
                    header.head {
                        a.mark href=(if viewer.is_signed_in() { "/" } else { "/explore" }) { span.dot {} "kvitter" }
                        nav {
                            @if let Some(user) = viewer.user() {
                                a href="/" aria-current=[(nav == Nav::Mine).then_some("page")] { "Mine" }
                                a href="/explore" aria-current=[(nav == Nav::Explore).then_some("page")] { "Explore" }
                                a.who href={ "/u/" (encode_path(user)) } {
                                    span.avatar aria-hidden="true" { (initial(user)) }
                                    (user)
                                }
                            } @else {
                                a href="/explore" aria-current=[(nav == Nav::Explore).then_some("page")] { "Explore" }
                            }
                        }
                    }
                    main { (content) }
                }
            }
        }
    }
}

fn initial(user: &str) -> String {
    user.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

pub fn icon(name: &str) -> Markup {
    html! { svg.i aria-hidden="true" { use href={ "#i-" (name) } {} } }
}

const ICONS: &str = r##"<svg class="sprite" width="0" height="0" aria-hidden="true">
<symbol id="i-lock" viewBox="0 0 16 16"><rect x="3" y="7" width="10" height="7" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.4"/><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" fill="none" stroke="currentColor" stroke-width="1.4"/></symbol>
<symbol id="i-globe" viewBox="0 0 16 16"><circle cx="8" cy="8" r="5.8" fill="none" stroke="currentColor" stroke-width="1.3"/><path d="M2.4 8h11.2M8 2.2c1.8 1.7 2.6 3.7 2.6 5.8S9.8 12.1 8 13.8C6.2 12.1 5.4 10.1 5.4 8S6.2 3.9 8 2.2Z" fill="none" stroke="currentColor" stroke-width="1.3"/></symbol>
<symbol id="i-image" viewBox="0 0 16 16"><rect x="2" y="3" width="12" height="10" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.3"/><circle cx="6" cy="6.5" r="1.2" fill="currentColor"/><path d="M2.5 12l3.5-3.5 2.5 2.5 2-2 3 3" fill="none" stroke="currentColor" stroke-width="1.3"/></symbol>
<symbol id="i-edit" viewBox="0 0 16 16"><path d="M10.5 2.8l2.7 2.7-7.6 7.6-3.2.5.5-3.2z" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linejoin="round"/></symbol>
<symbol id="i-link" viewBox="0 0 16 16"><path d="M6.8 9.2a2.6 2.6 0 0 0 3.7 0l2.2-2.2a2.6 2.6 0 0 0-3.7-3.7l-.9.9M9.2 6.8a2.6 2.6 0 0 0-3.7 0L3.3 9a2.6 2.6 0 0 0 3.7 3.7l.9-.9" fill="none" stroke="currentColor" stroke-width="1.3" stroke-linecap="round"/></symbol>
</svg>"##;

pub fn vis_pill(v: Visibility) -> Markup {
    match v {
        Visibility::Public => html! { span.vis.pub { (icon("globe")) "Public" } },
        Visibility::Private => html! { span.vis.priv { (icon("lock")) "Private" } },
    }
}

/// The composer form. Works as a plain textarea; `composer.js` upgrades it to CodeMirror.
pub fn composer(action: &str, body: &str, visibility: Visibility, submit: &str, error: Option<&str>) -> Markup {
    let public = visibility == Visibility::Public;
    html! {
        form.composer method="post" action=(action) {
            @if let Some(e) = error { p.error role="alert" { (e) } }
            textarea name="body" data-composer aria-label="Post" placeholder="What's on your mind?"
                maxlength=(MAX_POST_CHARS) rows="4" { (body) }
            div.cbar {
                button.icon-btn type="button" data-upload hidden { (icon("image")) "Image" }
                input type="file" accept="image/jpeg,image/png,image/webp" data-upload-input hidden;
                span.spacer {}
                span.hint { "Ctrl+Enter" }
                div.seg role="radiogroup" aria-label="Visibility" {
                    label { input type="radio" name="visibility" value="private" checked[!public]; (icon("lock")) "Private" }
                    label.pub { input type="radio" name="visibility" value="public" checked[public]; (icon("globe")) "Public" }
                }
                button.primary type="submit" data-submit=(submit) { (submit) }
            }
        }
    }
}

pub struct ItemOpts {
    pub show_author: bool,
}

pub fn post_item(state: &AppState, viewer: &Viewer, post: &Post, opts: &ItemOpts) -> Markup {
    let own = viewer.owns(&post.author);
    html! {
        article.post {
            div.meta {
                @if opts.show_author { (author_link(viewer, &post.author)) }
                a.when href={ "/p/" (post.public_id) } { (when(state, post.created_at, false)) }
                @if own { (vis_pill(post.visibility)) }
            }
            div.body { (body(viewer, &post.body)) }
        }
    }
}

fn author_link(viewer: &Viewer, author: &str) -> Markup {
    html! {
        @if viewer.is_signed_in() {
            a.author href={ "/u/" (encode_path(author)) } { (author) }
        } @else {
            span.author { (author) }
        }
    }
}

pub fn body(viewer: &Viewer, text: &str) -> Markup {
    PreEscaped(markdown::render(text, &RenderOpts { base: "", link_tags: viewer.is_signed_in() }))
}

pub fn posts_list(state: &AppState, viewer: &Viewer, posts: &[Post], opts: &ItemOpts, older: Option<String>, empty: &str) -> Markup {
    html! {
        div.posts {
            @for p in posts { (post_item(state, viewer, p, opts)) }
            @if posts.is_empty() { p.empty { (empty) } }
            @if let Some(href) = older { a.more href=(href) { "Older posts" } }
        }
    }
}

pub fn when(state: &AppState, ts: i64, full: bool) -> Markup {
    let utc = DateTime::<Utc>::from_timestamp(ts, 0).unwrap_or_default();
    let local = state.config.tz.from_utc_datetime(&utc.naive_utc());
    let now = state.config.tz.from_utc_datetime(&Utc::now().naive_utc());
    let label = if full {
        local.format("%a %-d %b %Y, %H:%M").to_string()
    } else if local.date_naive() == now.date_naive() {
        local.format("%H:%M").to_string()
    } else if local.date_naive() == (now - Duration::days(1)).date_naive() {
        local.format("Yesterday %H:%M").to_string()
    } else if local.year() == now.year() {
        local.format("%a %-d %b").to_string()
    } else {
        local.format("%-d %b %Y").to_string()
    };
    html! { time datetime=(utc.to_rfc3339()) { (label) } }
}

pub fn single_post(state: &AppState, viewer: &Viewer, post: &Post) -> Markup {
    let own = viewer.owns(&post.author);
    let public_url = &state.config.public_url;
    html! {
        @if viewer.is_signed_in() { a.back href="/" { "← Back to feed" } }
        article.single {
            div.meta {
                (author_link(viewer, &post.author))
                a.when href={ "/p/" (post.public_id) } { (when(state, post.created_at, true)) }
                @if own { (vis_pill(post.visibility)) }
            }
            div.body { (body(viewer, &post.body)) }
        }
        @if own {
            div.actions {
                div.act-row {
                    a.btn href={ "/p/" (post.public_id) "/edit" } { (icon("edit")) "Edit" }
                    form.seg method="post" action={ "/p/" (post.public_id) "/visibility" } aria-label="Visibility" {
                        button type="submit" name="visibility" value="private"
                            aria-pressed=(if post.is_public() { "false" } else { "true" }) { (icon("lock")) "Private" }
                        button.pub type="submit" name="visibility" value="public"
                            aria-pressed=(if post.is_public() { "true" } else { "false" }) { (icon("globe")) "Public" }
                    }
                }
                @if post.is_public() && !public_url.is_empty() {
                    @let url = format!("{public_url}/p/{}", post.public_id);
                    div.pubrow {
                        span.lbl { "Public link" }
                        div.act-row {
                            button.btn type="button" data-copy=(url) { (icon("link")) span { "Copy public URL" } }
                        }
                        span.u data-copy-text { (url) }
                    }
                }
            }
        } @else if !viewer.is_signed_in() {
            div.pubfoot { a href="/explore" { "More public posts →" } }
        }
    }
}

/// canonical and og tags. Only ever emitted for public posts.
pub fn public_post_meta(state: &AppState, post: &Post) -> Option<Markup> {
    if !post.is_public() {
        return None;
    }
    let public_url = &state.config.public_url;
    let analysis = markdown::analyze(&post.body);
    let title = format!("{} on kvitter", post.author);
    let description = markdown::excerpt(&post.body, 200);
    Some(html! {
        @if !public_url.is_empty() {
            link rel="canonical" href={ (public_url) "/p/" (post.public_id) };
            meta property="og:url" content={ (public_url) "/p/" (post.public_id) };
            @if let Some(img) = &analysis.first_image {
                meta property="og:image" content={ (public_url) "/files/" (img) };
            }
        }
        meta property="og:type" content="article";
        meta property="og:site_name" content="kvitter";
        meta property="og:title" content=(title);
        meta property="og:description" content=(description);
        meta name="description" content=(description);
    })
}

pub fn page_head(title: Markup, sub: Option<String>) -> Markup {
    html! {
        div.pagehead {
            h1 { (title) }
            @if let Some(s) = sub { p { (s) } }
        }
    }
}

pub fn edit_page(post: &Post, body: &str, visibility: Visibility, error: Option<&str>) -> Markup {
    html! {
        a.back href={ "/p/" (post.public_id) } { "← Back to post" }
        (composer(&format!("/p/{}/edit", post.public_id), body, visibility, "Save", error))
        details.danger {
            summary { "Delete post" }
            form method="post" action={ "/p/" (post.public_id) "/delete" } {
                p { "This removes the post and any images only it uses. It can't be undone." }
                button.btn.destructive type="submit" { "Delete permanently" }
            }
        }
    }
}

pub fn not_found_body() -> Markup {
    html! {
        div.pagehead {
            h1 { "Not found" }
            p { "There's nothing here, or it isn't yours to see." }
        }
    }
}
