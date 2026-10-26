//! Requests through the whole router: identity, the anonymous gate and every leak path in the brief.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use http_body_util::BodyExt;
use kvitter::{AppState, app, config::Config, db};
use tower::ServiceExt;

const PROOF: &str = "test-proof";
const PUBLIC_URL: &str = "https://kv.example";

struct Harness {
    app: Router,
    _uploads: tempfile::TempDir,
}

fn harness(public_url: &str) -> Harness {
    let uploads = tempfile::tempdir().unwrap();
    let config = Config {
        public_url: public_url.into(),
        proof: Some(PROOF.into()),
        data_dir: uploads.path().into(),
        uploads_dir: uploads.path().into(),
        port: 0,
        tz: chrono_tz::Tz::UTC,
        dev_user: None,
    };
    Harness { app: app(AppState::new(config, db::open_in_memory().unwrap())), _uploads: uploads }
}

struct Res {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

impl Harness {
    async fn send(&self, req: Request<Body>) -> Res {
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        Res { status, headers, body: String::from_utf8_lossy(&bytes).into() }
    }

    async fn get(&self, user: Option<&str>, path: &str) -> Res {
        self.send(request("GET", user, path).body(Body::empty()).unwrap()).await
    }

    async fn post_form(&self, user: Option<&str>, path: &str, form: &str) -> Res {
        let req = request("POST", user, path)
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(form.to_string()))
            .unwrap();
        self.send(req).await
    }

    /// Creates a post and returns its id.
    async fn create(&self, user: &str, body: &str, visibility: &str) -> String {
        let form = format!("body={}&visibility={visibility}", urlencode(body));
        let res = self.post_form(Some(user), "/posts", &form).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.body);
        let feed = self.get(Some(user), "/").await.body;
        let at = feed.find("href=\"/p/").expect("new post in feed") + "href=\"/p/".len();
        feed[at..at + 8].to_string()
    }

    async fn upload(&self, user: &str, bytes: &[u8]) -> String {
        let res = self.multipart(user, "/files", bytes).await;
        assert_eq!(res.status, StatusCode::OK, "{}", res.body);
        let json: serde_json::Value = serde_json::from_str(&res.body).unwrap();
        json["id"].as_str().unwrap().to_string()
    }

    /// Sets the user's avatar and returns its file id.
    async fn set_avatar(&self, user: &str, bytes: &[u8]) -> String {
        let res = self.multipart(user, "/settings/avatar", bytes).await;
        assert_eq!(res.status, StatusCode::SEE_OTHER, "{}", res.body);
        let page = self.get(Some(user), "/settings").await.body;
        let at = page.find("src=\"/files/").expect("avatar on settings page") + "src=\"/files/".len();
        page[at..at + 8].to_string()
    }

    async fn multipart(&self, user: &str, path: &str, bytes: &[u8]) -> Res {
        let boundary = "XBOUNDARYX";
        let mut body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\n"
        )
        .into_bytes();
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let req = request("POST", Some(user), path)
            .header(header::CONTENT_TYPE, format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(body))
            .unwrap();
        self.send(req).await
    }
}

fn request(method: &str, user: Option<&str>, path: &str) -> axum::http::request::Builder {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(u) = user {
        b = b.header("x-kvitter-user", u).header("x-kvitter-proof", PROOF);
    }
    b
}

fn urlencode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn png() -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 128]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

#[tokio::test]
async fn spoofed_user_header_is_anonymous() {
    let h = harness(PUBLIC_URL);
    let spoof = Request::get("/").header("x-kvitter-user", "max").body(Body::empty()).unwrap();
    let res = h.send(spoof).await;
    assert_eq!(res.status, StatusCode::SEE_OTHER);
    assert_eq!(res.headers[header::LOCATION], "/explore");

    let wrong = Request::get("/")
        .header("x-kvitter-user", "max")
        .header("x-kvitter-proof", "guess")
        .body(Body::empty())
        .unwrap();
    assert_eq!(h.send(wrong).await.status, StatusCode::SEE_OTHER);
    assert_eq!(h.get(Some("max"), "/").await.status, StatusCode::OK);
}

#[tokio::test]
async fn anonymous_is_read_only() {
    let h = harness(PUBLIC_URL);
    let id = h.create("max", "hello #sauna", "public").await;
    for path in ["/u/max", "/t/sauna", "/api/tags?q=", &format!("/p/{id}/edit")] {
        assert_eq!(h.get(None, path).await.status, StatusCode::NOT_FOUND, "{path}");
    }
    let res = h.post_form(None, "/posts", "body=hi&visibility=public").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    let res = h.post_form(None, &format!("/p/{id}/delete"), "").await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);

    let page = h.get(None, &format!("/p/{id}")).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(!page.body.contains("composer") && !page.body.contains("/edit"), "{}", page.body);
    assert!(page.body.contains("<span class=\"tag\">#sauna</span>"), "tags aren't links for anonymous");
}

#[tokio::test]
async fn private_posts_do_not_leak() {
    let h = harness(PUBLIC_URL);
    let public = h.create("max", "out loud #shared", "public").await;
    let private = h.create("max", "SECRET-TEXT #shared #diary", "private").await;

    // Missing and private look the same.
    let missing = h.get(Some("anna"), "/p/zzzzzzzz").await;
    let hidden = h.get(Some("anna"), &format!("/p/{private}")).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert_eq!(hidden.body, missing.body);
    assert_eq!(h.get(None, &format!("/p/{private}")).await.status, StatusCode::NOT_FOUND);
    assert_eq!(h.get(Some("max"), &format!("/p/{private}")).await.status, StatusCode::OK);

    for (user, path) in [
        (None, "/explore"),
        (None, "/rss.xml"),
        (None, "/sitemap.xml"),
        (Some("anna"), "/explore"),
        (Some("anna"), "/u/max"),
        (Some("anna"), "/t/shared"),
        (Some("max"), "/explore"),
        (Some("max"), "/rss.xml"),
        (Some("max"), "/sitemap.xml"),
    ] {
        let res = h.get(user, path).await;
        assert!(!res.body.contains("SECRET-TEXT") && !res.body.contains(&private), "{user:?} {path}");
    }
    assert!(h.get(None, "/rss.xml").await.body.contains(&public));
    assert!(h.get(None, "/sitemap.xml").await.body.contains(&format!("{PUBLIC_URL}/p/{public}")));

    // Tag pages, counts and autocomplete only count what the viewer can see.
    assert_eq!(h.get(Some("anna"), "/t/diary").await.status, StatusCode::NOT_FOUND);
    assert!(h.get(Some("anna"), "/t/shared").await.body.contains("1 post<"));
    assert!(h.get(Some("max"), "/t/shared").await.body.contains("2 posts<"));
    assert_eq!(h.get(Some("anna"), "/api/tags?q=").await.body, r#"[{"count":1,"tag":"shared"}]"#);
    assert!(h.get(Some("max"), "/api/tags?q=di").await.body.contains("diary"));

    // A user with only private posts doesn't exist for others.
    h.create("solo", "only me", "private").await;
    assert_eq!(h.get(Some("anna"), "/u/solo").await.status, StatusCode::NOT_FOUND);
    assert_eq!(h.get(Some("solo"), "/u/solo").await.status, StatusCode::OK);

    // og tags and canonical only on public posts.
    let own_private = h.get(Some("max"), &format!("/p/{private}")).await.body;
    assert!(!own_private.contains("og:") && !own_private.contains("canonical"));
    let pub_page = h.get(None, &format!("/p/{public}")).await.body;
    assert!(pub_page.contains(&format!("<link rel=\"canonical\" href=\"{PUBLIC_URL}/p/{public}\">")));
}

#[tokio::test]
async fn others_cannot_change_posts() {
    let h = harness(PUBLIC_URL);
    let id = h.create("max", "mine", "private").await;
    for path in [format!("/p/{id}/visibility"), format!("/p/{id}/edit"), format!("/p/{id}/delete")] {
        let res = h.post_form(Some("anna"), &path, "visibility=public&body=hacked").await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!(h.get(Some("anna"), &format!("/p/{id}")).await.status, StatusCode::NOT_FOUND);
    assert!(h.get(Some("max"), &format!("/p/{id}")).await.body.contains("mine"));
}

#[tokio::test]
async fn files_follow_post_visibility() {
    let h = harness(PUBLIC_URL);
    let file = h.upload("max", &png()).await;
    let path = format!("/files/{file}");
    assert_eq!(h.get(Some("max"), &path).await.status, StatusCode::OK, "owner sees unattached upload");
    assert_eq!(h.get(None, &path).await.status, StatusCode::NOT_FOUND);
    assert_eq!(h.get(Some("anna"), &path).await.status, StatusCode::NOT_FOUND);

    let id = h.create("max", &format!("lake ![]({path})"), "public").await;
    let res = h.get(None, &path).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.headers[header::CONTENT_TYPE], "image/png");
    assert_eq!(res.headers[header::CACHE_CONTROL], "public, no-cache");

    // Flipping the post private hides the file immediately.
    let res = h.post_form(Some("max"), &format!("/p/{id}/visibility"), "visibility=private").await;
    assert_eq!(res.status, StatusCode::SEE_OTHER);
    assert_eq!(h.get(None, &path).await.status, StatusCode::NOT_FOUND);
    assert_eq!(h.get(Some("anna"), &path).await.status, StatusCode::NOT_FOUND);

    // Linking someone else's file doesn't publish it.
    h.create("anna", &format!("look ![]({path})"), "public").await;
    assert_eq!(h.get(None, &path).await.status, StatusCode::NOT_FOUND);

    // Deleting the post removes files only it used.
    assert_eq!(h.post_form(Some("max"), &format!("/p/{id}/delete"), "").await.status, StatusCode::SEE_OTHER);
    assert_eq!(h.get(Some("max"), &path).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn caching_headers() {
    let h = harness(PUBLIC_URL);
    let id = h.create("max", "hi", "public").await;
    let signed_in = h.get(Some("max"), &format!("/p/{id}")).await;
    assert_eq!(signed_in.headers[header::CACHE_CONTROL], "private, no-store");
    let anon = h.get(None, &format!("/p/{id}")).await;
    assert_eq!(anon.headers[header::CACHE_CONTROL], "public, max-age=60");
    let feed = h.get(Some("max"), "/rss.xml").await;
    assert_eq!(feed.headers[header::CACHE_CONTROL], "private, no-store");
}

#[tokio::test]
async fn cross_site_posts_are_refused() {
    let h = harness(PUBLIC_URL);
    for site in ["cross-site", "same-site"] {
        let req = request("POST", Some("max"), "/posts")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("sec-fetch-site", site)
            .body(Body::from("body=csrf&visibility=public"))
            .unwrap();
        assert_eq!(h.send(req).await.status, StatusCode::FORBIDDEN, "{site}");
    }
    assert!(!h.get(Some("max"), "/").await.body.contains("csrf"));
}

#[tokio::test]
async fn post_validation() {
    let h = harness(PUBLIC_URL);
    let res = h.post_form(Some("max"), "/posts", "body=%20%20&visibility=public").await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(res.body.contains("Write something first."));

    let long = "a".repeat(100_001);
    let res = h.post_form(Some("max"), "/posts", &format!("body={long}&visibility=private")).await;
    assert_eq!(res.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(res.body.contains("at most 100,000"));

    // Unknown visibility falls back to private.
    let id = h.create("max", "hmm", "everyone").await;
    assert_eq!(h.get(Some("anna"), &format!("/p/{id}")).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn without_public_url() {
    let h = harness("");
    let id = h.create("max", "hello", "public").await;
    let page = h.get(Some("max"), &format!("/p/{id}")).await.body;
    assert!(!page.contains("Copy public URL"));
    assert!(!page.contains("canonical") && !page.contains("og:url"));
    assert_eq!(h.get(None, "/rss.xml").await.status, StatusCode::NOT_FOUND);
    assert_eq!(h.get(None, "/sitemap.xml").await.status, StatusCode::NOT_FOUND);
    assert_eq!(h.get(None, "/robots.txt").await.body, "User-agent: *\nDisallow: /\n");

    let h = harness(PUBLIC_URL);
    let id = h.create("max", "hello", "public").await;
    let page = h.get(Some("max"), &format!("/p/{id}")).await.body;
    assert!(page.contains(&format!("data-copy=\"{PUBLIC_URL}/p/{id}\"")));
    let private = h.create("max", "hush", "private").await;
    assert!(!h.get(Some("max"), &format!("/p/{private}")).await.body.contains("Copy public URL"));
}

#[tokio::test]
async fn avatars_show_only_where_their_owner_does() {
    let h = harness(PUBLIC_URL);
    let first = h.set_avatar("max", &png()).await;
    h.create("max", "only private so far", "private").await;
    // max has nothing public, so his avatar (like his name) is unknown to others.
    for user in [None, Some("anna")] {
        assert_eq!(h.get(user, &format!("/files/{first}")).await.status, StatusCode::NOT_FOUND);
    }
    assert_eq!(h.get(None, "/settings").await.status, StatusCode::NOT_FOUND);

    let public = h.create("max", "hello", "public").await;
    assert_eq!(h.get(None, &format!("/files/{first}")).await.status, StatusCode::OK);
    assert!(h.get(None, &format!("/p/{public}")).await.body.contains(&format!("/files/{first}")));
    assert!(h.get(Some("anna"), "/explore").await.body.contains(&format!("/files/{first}")));

    // Replacing it removes the old one; deleting a post that embeds the avatar keeps it.
    let second = h.set_avatar("max", &png()).await;
    assert_ne!(first, second);
    assert_eq!(h.get(Some("max"), &format!("/files/{first}")).await.status, StatusCode::NOT_FOUND);
    let embeds = h.create("max", &format!("![](/files/{second})"), "private").await;
    assert_eq!(h.post_form(Some("max"), &format!("/p/{embeds}/delete"), "").await.status, StatusCode::SEE_OTHER);
    assert_eq!(h.get(None, &format!("/files/{second}")).await.status, StatusCode::OK);

    assert_eq!(h.post_form(Some("max"), "/settings/avatar/delete", "").await.status, StatusCode::SEE_OTHER);
    assert_eq!(h.get(Some("max"), &format!("/files/{second}")).await.status, StatusCode::NOT_FOUND);
}
