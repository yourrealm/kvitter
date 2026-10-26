# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`brief.md` is the product spec; read it before design decisions. Realm platform docs: https://yourrealm.eu/docs/microapps/ and `/docs/realm-ts/*`.

## What Kvitter is

A small Twitter-like app for short posts, packaged as a **Realm micro app**. One Rust binary (axum, rusqlite, maud templates), server-rendered HTML for every page, SQLite on `/data` and uploads on `/uploads`. JS only for the composer (CodeMirror 6, bundled from `web/composer.js`) and `static/app.js` (Copy public URL, button labels).

## Commands

```sh
pnpm install
pnpm build:web                 # bundles web/composer.js -> static/composer.js; needed before any cargo build
pnpm dev                       # local server on :3000, see "Local dev" below
cargo test                     # unit tests + tests/http.rs (leak tests through the whole router)
cargo test --test http private_posts_do_not_leak   # a single test
pnpm test:e2e                  # Playwright: builds, starts its own server on :3100, desktop + phone
pnpm test:e2e -g "edit and delete" --project desktop
docker build -t kvitter .      # the image Forge builds (kaniko: no BuildKit-only features)
```

`static/composer.js` is embedded with `include_bytes!`, so `cargo build` fails until `pnpm build:web` has run once.

Realm recipe (needs Deno; `realm-sdk` isn't installed here yet): `realm-sdk validate realm.ts`. Deploy: Forge builds the repo/branch bound on its page on every push.

### Local dev

There's no Realm locally. `kvitter serve --dev-user max` (what `pnpm dev` runs) signs every request in as `max`, **except** on hosts starting with `public.`, which are anonymous. So `http://localhost:3000` is the private URL and `http://public.localhost:3000` is the public domain. Other users: send `X-Kvitter-User` + `X-Kvitter-Proof` with `KVITTER_PROOF` set and no `--dev-user`, as the e2e fixtures do.

Env: `PUBLIC_URL`, `KVITTER_PROOF`, `KVITTER_TZ` (IANA, default UTC), `DATA_DIR` (/data), `UPLOADS_DIR` (/uploads), `PORT` (3000).

## Architecture

Request flow: `auth::identify` (middleware on every route) → handler in `routes.rs` / `feeds.rs` / `statics.rs` → `views.rs` (maud).

- `auth.rs` turns headers into a `Viewer` (`Anonymous` or `User`), trusting `X-Kvitter-User` only when `X-Kvitter-Proof` matches `KVITTER_PROOF`. It also enforces the anonymous route allowlist (that's what makes the public domain read-only: anonymous requests only arrive there), refuses cross-site/same-site POSTs via `Sec-Fetch-Site`, sets `Cache-Control`, and sends the CSP with a per-response nonce (task-local, read by `views::layout` for CodeMirror's injected styles).
- `visibility.rs` is **the one visibility filter**: `posts()` and `files()` return SQL conditions. Every query in `db.rs` that returns posts, tags, counts or files composes them; never write a visibility condition inline. Writes check `author = viewer` themselves.
- `db.rs`: schema + migrations via `PRAGMA user_version`. Internal `INTEGER PRIMARY KEY`s; URLs use 8-char random `public_id`s (`ids.rs`), retried on unique conflict. Tags and `post_files` are re-derived from the body on every save (`index_post`). A file attaches only to posts by its owner. A file is visible to its owner or if any attached post is visible.
- `markdown.rs`: pulldown-cmark with raw HTML turned into text, unsafe link schemes dropped, only `/files/<id>` rendered as images, and `#tags`/bare URLs found by `segments()`. The tag rule must match the regex in `web/composer.js`. Anonymous pages render tags as text because `/t/` isn't served to them.
- `images.rs`: decode, apply EXIF orientation, scale to ≤2048 px, re-encode (JPEG, or PNG if alpha). Re-encoding is what strips EXIF.
- `feeds.rs`: RSS/sitemap always query as `Viewer::Anonymous`, and 404 without `PUBLIC_URL`.

Privacy rules worth remembering (full list in `brief.md` under "No leaks"): a private post and a missing one return the identical 404; a user with no posts visible to you is a 404; `PUBLIC_URL` is never redirected to and never replaced by the private URL; og/canonical tags only on public posts.

## Tests

- `tests/http.rs` drives the full router with `tower::ServiceExt::oneshot`. Add a case there for any new route that returns posts, tags or files.
- `e2e/` uses fixtures `max`, `anna` (signed in via headers) and `anon`. Every test fails on any CSP violation (`fixtures.ts`). Tests share one server and database, so use `unique()` text rather than assuming an empty feed.

## Realm platform facts

- Realm injects no env vars; the Dockerfile sets `PORT=3000`. The healthcheck must be exec form (`/app/kvitter healthcheck`); distroless has no shell.
- The ingress strips client-sent copies of headers declared in `getTrustedHeaders`, on the private subdomain and public domains.
- One backup strategy per volume, hence separate `/data` (sqlite) and `/uploads` (file-copy) volumes.
- Router defaults: `maxRequestSizeMb` 4 (Kvitter sets 25; `lib.rs` `UPLOAD_LIMIT` must match), rate limit 1000 per 10 s. Realm sets HSTS, X-Frame-Options and nosniff, but no CSP.
