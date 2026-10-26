# Kvitter

A small, Twitter-like place for short posts, as a Realm micro app. Replaces Memos for Max. Server-rendered so public posts get indexed, private by default, web only.

## Stack

- Rust, axum, rusqlite. One binary, one service, distroless image (`gcr.io/distroless/cc-debian12`), healthcheck via a `healthcheck` subcommand.
- Server-rendered HTML for every page. JS only where it earns it: the composer (CodeMirror 6, markdown kept verbatim, styled in place) and small enhancements like "Copy public URL". Without JS the composer is a plain textarea and everything except image upload still works.
- The app sends its own Content-Security-Policy, not the Realm router: CodeMirror injects `<style>` elements, which need a per-response nonce.
- SQLite on one volume (`/data`, sqlite backup). Uploads on their own volume (`/uploads`, file-copy backup): Realm allows one backup kind per volume, and the sqlite kind covers only the db file.
- Uploads stay on the volume, not S3: every file goes through the visibility check anyway, and the volume gets Realm's backups for free.

## Identity

- Realm's gate maps the signed-in user to `X-Kvitter-User` via `getTrustedHeaders`.
- Realm also sends `X-Kvitter-Proof`, a stable secret (`r.stableSecret("home-proof")`) that the app gets as `KVITTER_PROOF`. The user header is trusted only when the proof header matches it (constant-time compare). No match, or `KVITTER_PROOF` unset: anonymous.
- Header present (with valid proof): signed in as that user. Header absent: anonymous.
- Realm's ingress already strips client-sent copies of both headers on every route, the public domain included. The proof guards against anything that reaches the container past the ingress.
- No bearer-token API, no native clients. If a phone app ever happens, it uses Realm's native sign-in and needs no server changes.

## Visibility

Every post is `public` or `private`.

| Viewer | Sees |
| --- | --- |
| Anonymous | Public posts only |
| Signed in | Own posts (public and private) + other users' public posts |

- Signed in, the default feed is **my own posts**. Other people's public posts live at `/explore`, not the default.
- New posts always default to private. In the UI the public marker is the louder one; private is the quiet norm.
- Every query that returns posts, tags, counts or files goes through one visibility filter. No query builds its own.

## No leaks

Private posts must not leak through anything around them:

- **Tags**: tag lists, tag counts, autocomplete, tag pages, search and feeds only count posts the viewer can see.
- **Files**: a file is served only if its post is visible to the viewer. Flipping a post to private hides its files immediately. Unguessable paths are not the check.
- **EXIF**: strip metadata (GPS especially) from images on upload by decoding and re-encoding them. The same step scales them down to at most 2048 px on the long side.
- **Feeds and SEO**: RSS, sitemap, og tags and search only ever contain public posts, even when requested signed in. Their links use `PUBLIC_URL`.
- **Caching**: pages rendered for a signed-in user send `Cache-Control: private, no-store`. Only anonymous public pages may be cached (60 s). Files are served `public, no-cache` with an ETag, so a cache must check before reusing one.
- **Files and other people's posts**: a file attaches only to posts by its owner. Linking someone else's upload in your public post doesn't publish it.

## Public URL

Realm serves the app on two URLs: the private one behind Home's login (e.g. `https://kvitter-xxx-web.w1.ok2m.yourrealm.me`) and an optional public domain.

- `PUBLIC_URL` comes in through the env, derived in `realm.ts` from the public domain (`https://` + `config.domain.hosts[0]`, empty when no domain is set). The app **never redirects** to it. Browsing and writing happen on the private URL, signed in.
- "Copy public URL" on a public post copies `${PUBLIC_URL}/p/:id`.
- `<link rel="canonical">` and `og:url` always use `PUBLIC_URL`.
- If `PUBLIC_URL` is empty, hide "Copy public URL" instead of falling back to the private URL. `/rss.xml` and `/sitemap.xml` then 404, and `robots.txt` disallows everything.
- The public domain is read-only: no composer, no edit, no upload.
- The app never checks which domain a request came in on; it decides by identity. The private URL is behind Home's login, so anonymous requests only arrive via the public domain.
- Anonymous visitors get only `/explore`, `/p/:id`, `/files/:id` (for images in public posts), `/rss.xml`, `/sitemap.xml`, `/robots.txt`, plus `/static/*` (CSS, favicon) and `/healthz`. `/` redirects them to `/explore` (a relative redirect, never to `PUBLIC_URL`). Everything else 404s.
- "Copy public URL" is hidden on private posts (no "make public and copy" shortcut).

## Pages

- `/` own posts (anonymous: redirect to `/explore`)
- `/explore` everyone's public posts
- `/p/:id` single post, SSR with og tags
- `/p/:id/edit` edit page (separate page, not in place); delete is available
- `/u/:name` a user's visible posts
- `/t/:tag` posts with a tag, visibility-filtered
- `/files/:id` uploads, behind the visibility check
- `/settings` profile picture (signed-in only)
- `/rss.xml`, `/sitemap.xml`, `/robots.txt`

## Posts

- Max length 100,000 characters.
- Markdown is rendered on the server. Raw HTML in a post is escaped and shown as text, never rendered.
- Tags: `#` followed by letters, digits, `_` or `-` (Unicode letters allowed), with at least one letter (so "issue #1" isn't a tag), stored lowercase. A `#` inside code or a link doesn't make a tag.
- Line breaks in a post are kept. Links keep only http(s), mailto and same-site paths. Only uploads render as images.
- Post times are shown in `KVITTER_TZ` (an IANA name, default UTC).

## Profile pictures

- Optional, set on `/settings`; cropped to a 256 px square. Shown next to every post, the user's initial otherwise.
- Visible exactly where the user's name is: to their owner, and to anyone who can see at least one of their posts.

## Importing from memos

- `kvitter import-memos MEMOS_DB --as USER`. Only memos that are public and not archived become public; everything else (private, protected, archived) becomes private. Original times are kept, attachments become uploads appended to the post.
- Re-runnable: imported memos are recorded in `imports` and skipped, including ones deleted since.

## IDs

- Posts and files get a random 8-character public id, e.g. `/p/k7mq3xzh`. Alphabet: lowercase letters and digits without look-alikes (`0 o 1 l i`), 31 characters, ~850 billion ids.
- Random, not sequential: sequential ids reveal how many private posts exist and when. No sqids/hashids either, since they encode the sequence.
- 8 rather than 5–6 so that an "unlisted" visibility stays safe to add later: a guesser at the router's 100 req/s would need years to find one.
- SQLite keeps an `INTEGER PRIMARY KEY` for joins; the public id is a separate `TEXT UNIQUE` column. Insert, and retry with a fresh id on a unique conflict.
- Files get their own ids, not ids derived from the post's.
- A private post and a missing one return the identical 404.

## Out of scope

- Threads and replies.
- Bearer tokens / API for scripts.

## realm.ts

See `realm.ts` at the repo root.

## Open, not decided

- Unlisted as a third visibility (link-only, not in feeds, sitemap or search, `noindex`).
- Pinned posts.
- Search (SQLite FTS5, visibility-filtered).
- Markdown export.
