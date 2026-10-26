//! SQLite storage. Every read that returns posts, tags, counts or files goes through
//! [`crate::visibility`]; writes check ownership themselves.

use rusqlite::{Connection, ErrorCode, OptionalExtension, Row, params, params_from_iter, types::Value};

use crate::{ids, markdown::Analysis, visibility::{self, Viewer}};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Public,
    Private,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "public" => Some(Visibility::Public),
            "private" => Some(Visibility::Private),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Post {
    pub id: i64,
    pub public_id: String,
    pub author: String,
    pub body: String,
    pub visibility: Visibility,
    pub created_at: i64,
    pub updated_at: i64,
    /// The author's avatar file, if they have one.
    pub avatar: Option<String>,
}

impl Post {
    pub fn is_public(&self) -> bool {
        self.visibility == Visibility::Public
    }

    fn from_row(row: &Row) -> rusqlite::Result<Self> {
        let vis: String = row.get("visibility")?;
        Ok(Post {
            id: row.get("id")?,
            public_id: row.get("public_id")?,
            author: row.get("author")?,
            body: row.get("body")?,
            visibility: Visibility::parse(&vis).unwrap_or(Visibility::Private),
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            avatar: row.get("avatar")?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct StoredFile {
    pub public_id: String,
    pub name: String,
    pub content_type: String,
}

/// Which posts a list shows, on top of what the viewer may see.
pub enum Scope<'a> {
    /// The viewer's own posts (the signed-in home feed).
    Own,
    /// Public posts by everyone (`/explore`, feeds, sitemap).
    Public,
    Author(&'a str),
    Tag(&'a str),
}

/// Position after the last post shown, for "Older posts".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub created_at: i64,
    pub id: i64,
}

impl Cursor {
    pub fn of(post: &Post) -> Self {
        Cursor { created_at: post.created_at, id: post.id }
    }

    pub fn encode(self) -> String {
        format!("{}-{}", self.created_at, self.id)
    }

    pub fn decode(s: &str) -> Option<Self> {
        let (a, b) = s.split_once('-')?;
        Some(Cursor { created_at: a.parse().ok()?, id: b.parse().ok()? })
    }
}

pub fn open(path: &std::path::Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    Ok(conn)
}

pub fn open_in_memory() -> rusqlite::Result<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    Ok(conn)
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "wal")?;
    conn.pragma_update(None, "foreign_keys", "on")?;
    conn.pragma_update(None, "busy_timeout", 5000)?;
    migrate(conn)
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if version < 1 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE posts (
               id INTEGER PRIMARY KEY,
               public_id TEXT NOT NULL UNIQUE,
               author TEXT NOT NULL,
               body TEXT NOT NULL,
               visibility TEXT NOT NULL CHECK (visibility IN ('public', 'private')),
               created_at INTEGER NOT NULL,
               updated_at INTEGER NOT NULL
             );
             CREATE INDEX posts_author_created ON posts (author, created_at DESC, id DESC);
             CREATE INDEX posts_visibility_created ON posts (visibility, created_at DESC, id DESC);
             CREATE TABLE tags (
               post_id INTEGER NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
               tag TEXT NOT NULL,
               PRIMARY KEY (post_id, tag)
             );
             CREATE INDEX tags_tag ON tags (tag);
             CREATE TABLE files (
               id INTEGER PRIMARY KEY,
               public_id TEXT NOT NULL UNIQUE,
               owner TEXT NOT NULL,
               name TEXT NOT NULL,
               content_type TEXT NOT NULL,
               created_at INTEGER NOT NULL
             );
             CREATE TABLE post_files (
               post_id INTEGER NOT NULL REFERENCES posts (id) ON DELETE CASCADE,
               file_id INTEGER NOT NULL REFERENCES files (id) ON DELETE CASCADE,
               PRIMARY KEY (post_id, file_id)
             );
             CREATE INDEX post_files_file ON post_files (file_id);
             PRAGMA user_version = 1;
             COMMIT;",
        )?;
    }
    if version < 2 {
        // What `kvitter import-*` has brought in, so re-running skips it. No foreign key:
        // a post deleted after import stays deleted.
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE imports (source TEXT PRIMARY KEY, post_public_id TEXT NOT NULL);
             PRAGMA user_version = 2;
             COMMIT;",
        )?;
    }
    if version < 3 {
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE avatars (user TEXT PRIMARY KEY, file_id INTEGER NOT NULL REFERENCES files (id));
             CREATE INDEX avatars_file ON avatars (file_id);
             PRAGMA user_version = 3;
             COMMIT;",
        )?;
    }
    Ok(())
}

/// Every post query selects these, so `Post::from_row` gets the author's avatar too.
const POST_COLUMNS: &str =
    "p.*, (SELECT af.public_id FROM avatars a JOIN files af ON af.id = a.file_id WHERE a.user = p.author) AS avatar";

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn is_unique_violation(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == ErrorCode::ConstraintViolation)
}

/// Run `insert` with fresh ids until one doesn't collide.
fn with_new_id<T>(mut insert: impl FnMut(&str) -> rusqlite::Result<T>) -> rusqlite::Result<T> {
    let mut attempts = 0;
    loop {
        match insert(&ids::new_id()) {
            Err(e) if is_unique_violation(&e) && attempts < 5 => attempts += 1,
            other => return other,
        }
    }
}

/// Tags and file attachments follow the body. Files attach only when the post's author owns
/// them, so nobody can publish someone else's private upload by linking to it.
fn index_post(conn: &Connection, post_id: i64, author: &str, analysis: &Analysis) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM tags WHERE post_id = ?", [post_id])?;
    conn.execute("DELETE FROM post_files WHERE post_id = ?", [post_id])?;
    for tag in &analysis.tags {
        conn.execute("INSERT OR IGNORE INTO tags (post_id, tag) VALUES (?, ?)", params![post_id, tag])?;
    }
    for file in &analysis.file_ids {
        conn.execute(
            "INSERT OR IGNORE INTO post_files (post_id, file_id)
             SELECT ?, id FROM files WHERE public_id = ? AND owner = ?",
            params![post_id, file, author],
        )?;
    }
    Ok(())
}

pub fn create_post(
    conn: &mut Connection,
    author: &str,
    body: &str,
    visibility: Visibility,
    analysis: &Analysis,
) -> rusqlite::Result<String> {
    let tx = conn.transaction()?;
    let ts = now();
    let public_id = insert_post(&tx, author, body, visibility, (ts, ts), analysis)?;
    tx.commit()?;
    Ok(public_id)
}

fn insert_post(
    tx: &Connection,
    author: &str,
    body: &str,
    visibility: Visibility,
    (created_at, updated_at): (i64, i64),
    analysis: &Analysis,
) -> rusqlite::Result<String> {
    let (post_id, public_id) = with_new_id(|id| {
        tx.execute(
            "INSERT INTO posts (public_id, author, body, visibility, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![id, author, body, visibility.as_str(), created_at, updated_at],
        )?;
        Ok((tx.last_insert_rowid(), id.to_string()))
    })?;
    index_post(tx, post_id, author, analysis)?;
    Ok(public_id)
}

pub fn is_imported(conn: &Connection, source: &str) -> rusqlite::Result<bool> {
    conn.query_row("SELECT 1 FROM imports WHERE source = ?", [source], |_| Ok(())).optional().map(|r| r.is_some())
}

/// Like [`create_post`], with the original times, recorded under `source` for [`is_imported`].
pub fn import_post(
    conn: &mut Connection,
    source: &str,
    author: &str,
    body: &str,
    visibility: Visibility,
    times: (i64, i64),
    analysis: &Analysis,
) -> rusqlite::Result<String> {
    let tx = conn.transaction()?;
    let public_id = insert_post(&tx, author, body, visibility, times, analysis)?;
    tx.execute("INSERT INTO imports (source, post_public_id) VALUES (?, ?)", params![source, public_id])?;
    tx.commit()?;
    Ok(public_id)
}

/// Returns false when the post doesn't exist or isn't the viewer's.
pub fn update_post(
    conn: &mut Connection,
    viewer: &Viewer,
    public_id: &str,
    body: &str,
    visibility: Visibility,
    analysis: &Analysis,
) -> rusqlite::Result<bool> {
    let Some(author) = viewer.user() else { return Ok(false) };
    let tx = conn.transaction()?;
    let post_id: Option<i64> = tx
        .query_row("SELECT id FROM posts WHERE public_id = ? AND author = ?", params![public_id, author], |r| r.get(0))
        .optional()?;
    let Some(post_id) = post_id else { return Ok(false) };
    tx.execute(
        "UPDATE posts SET body = ?, visibility = ?, updated_at = ? WHERE id = ?",
        params![body, visibility.as_str(), now(), post_id],
    )?;
    index_post(&tx, post_id, author, analysis)?;
    tx.commit()?;
    Ok(true)
}

pub fn set_visibility(conn: &Connection, viewer: &Viewer, public_id: &str, visibility: Visibility) -> rusqlite::Result<bool> {
    let Some(author) = viewer.user() else { return Ok(false) };
    let n = conn.execute(
        "UPDATE posts SET visibility = ?, updated_at = ? WHERE public_id = ? AND author = ?",
        params![visibility.as_str(), now(), public_id, author],
    )?;
    Ok(n == 1)
}

/// Deletes the viewer's post. Returns the stored names of files no other post uses any more,
/// which have been removed from the database and should be removed from disk; `None` when the
/// post doesn't exist or isn't the viewer's.
pub fn delete_post(conn: &mut Connection, viewer: &Viewer, public_id: &str) -> rusqlite::Result<Option<Vec<String>>> {
    let Some(author) = viewer.user() else { return Ok(None) };
    let tx = conn.transaction()?;
    let post_id: Option<i64> = tx
        .query_row("SELECT id FROM posts WHERE public_id = ? AND author = ?", params![public_id, author], |r| r.get(0))
        .optional()?;
    let Some(post_id) = post_id else { return Ok(None) };
    let orphans: Vec<(i64, String)> = tx
        .prepare(
            "SELECT f.id, f.name FROM files f JOIN post_files pf ON pf.file_id = f.id
             WHERE pf.post_id = ?1
               AND NOT EXISTS (SELECT 1 FROM post_files o WHERE o.file_id = f.id AND o.post_id != ?1)
               AND NOT EXISTS (SELECT 1 FROM avatars a WHERE a.file_id = f.id)",
        )?
        .query_map([post_id], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    tx.execute("DELETE FROM posts WHERE id = ?", [post_id])?;
    for (id, _) in &orphans {
        tx.execute("DELETE FROM files WHERE id = ?", [id])?;
    }
    tx.commit()?;
    Ok(Some(orphans.into_iter().map(|(_, name)| name).collect()))
}

pub fn get_post(conn: &Connection, viewer: &Viewer, public_id: &str) -> rusqlite::Result<Option<Post>> {
    let f = visibility::posts(viewer, "p");
    let mut params = vec![Value::Text(public_id.to_string())];
    params.extend(f.params);
    conn.query_row(
        &format!("SELECT {POST_COLUMNS} FROM posts p WHERE p.public_id = ? AND {}", f.sql),
        params_from_iter(params),
        Post::from_row,
    )
    .optional()
}

fn scope_condition(viewer: &Viewer, scope: &Scope) -> (String, Vec<Value>) {
    match scope {
        // Anonymous viewers have no posts of their own; match nothing rather than everything.
        Scope::Own => match viewer.user() {
            Some(u) => ("p.author = ?".into(), vec![Value::Text(u.to_string())]),
            None => ("0".into(), vec![]),
        },
        Scope::Public => ("p.visibility = 'public'".into(), vec![]),
        Scope::Author(a) => ("p.author = ?".into(), vec![Value::Text(a.to_string())]),
        Scope::Tag(t) => (
            "EXISTS (SELECT 1 FROM tags st WHERE st.post_id = p.id AND st.tag = ?)".into(),
            vec![Value::Text(t.to_string())],
        ),
    }
}

pub fn list_posts(
    conn: &Connection,
    viewer: &Viewer,
    scope: &Scope,
    before: Option<Cursor>,
    limit: usize,
) -> rusqlite::Result<Vec<Post>> {
    let f = visibility::posts(viewer, "p");
    let (scope_sql, scope_params) = scope_condition(viewer, scope);
    let mut sql = format!("SELECT {POST_COLUMNS} FROM posts p WHERE {} AND {scope_sql}", f.sql);
    let mut params = f.params;
    params.extend(scope_params);
    if let Some(c) = before {
        sql.push_str(" AND (p.created_at < ? OR (p.created_at = ? AND p.id < ?))");
        params.extend([Value::Integer(c.created_at), Value::Integer(c.created_at), Value::Integer(c.id)]);
    }
    sql.push_str(" ORDER BY p.created_at DESC, p.id DESC LIMIT ?");
    params.push(Value::Integer(limit as i64));
    conn.prepare(&sql)?.query_map(params_from_iter(params), Post::from_row)?.collect()
}

pub struct Counts {
    pub total: i64,
    pub public: i64,
}

pub fn count_posts(conn: &Connection, viewer: &Viewer, scope: &Scope) -> rusqlite::Result<Counts> {
    let f = visibility::posts(viewer, "p");
    let (scope_sql, scope_params) = scope_condition(viewer, scope);
    let mut params = f.params;
    params.extend(scope_params);
    conn.query_row(
        &format!(
            "SELECT COUNT(*), COALESCE(SUM(p.visibility = 'public'), 0) FROM posts p WHERE {} AND {scope_sql}",
            f.sql
        ),
        params_from_iter(params),
        |r| Ok(Counts { total: r.get(0)?, public: r.get(1)? }),
    )
}

/// Tags on posts the viewer can see, most used first, for autocomplete.
pub fn tags_with_prefix(conn: &Connection, viewer: &Viewer, prefix: &str, limit: usize) -> rusqlite::Result<Vec<(String, i64)>> {
    let f = visibility::posts(viewer, "p");
    let escaped = prefix.to_lowercase().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    let mut params = f.params;
    params.push(Value::Text(format!("{escaped}%")));
    params.push(Value::Integer(limit as i64));
    conn.prepare(&format!(
        "SELECT t.tag, COUNT(*) AS n FROM tags t JOIN posts p ON p.id = t.post_id
         WHERE {} AND t.tag LIKE ? ESCAPE '\\'
         GROUP BY t.tag ORDER BY n DESC, t.tag LIMIT ?",
        f.sql
    ))?
    .query_map(params_from_iter(params), |r| Ok((r.get(0)?, r.get(1)?)))?
    .collect()
}

pub fn create_file(conn: &Connection, owner: &str, ext: &str, content_type: &str) -> rusqlite::Result<String> {
    with_new_id(|id| {
        conn.execute(
            "INSERT INTO files (public_id, owner, name, content_type, created_at) VALUES (?, ?, ?, ?, ?)",
            params![id, owner, format!("{id}.{ext}"), content_type, now()],
        )?;
        Ok(id.to_string())
    })
}

/// The viewer's own avatar, for the header.
pub fn avatar_of(conn: &Connection, user: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT f.public_id FROM avatars a JOIN files f ON f.id = a.file_id WHERE a.user = ?",
        [user],
        |r| r.get(0),
    )
    .optional()
}

/// Sets (or with `None`, removes) the user's avatar to one of their files. Returns the file
/// name of the previous avatar when nothing else uses it, for removal from disk.
pub fn set_avatar(conn: &mut Connection, user: &str, file: Option<&str>) -> rusqlite::Result<Option<String>> {
    let tx = conn.transaction()?;
    let old: Option<(i64, String)> = tx
        .query_row(
            "SELECT f.id, f.name FROM avatars a JOIN files f ON f.id = a.file_id WHERE a.user = ?",
            [user],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    tx.execute("DELETE FROM avatars WHERE user = ?", [user])?;
    if let Some(file) = file {
        tx.execute(
            "INSERT INTO avatars (user, file_id) SELECT ?1, id FROM files WHERE public_id = ?2 AND owner = ?1",
            params![user, file],
        )?;
    }
    let mut removed = None;
    if let Some((id, name)) = old {
        let used: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM post_files WHERE file_id = ?1) OR EXISTS (SELECT 1 FROM avatars WHERE file_id = ?1)",
            [id],
            |r| r.get(0),
        )?;
        if !used {
            tx.execute("DELETE FROM files WHERE id = ?", [id])?;
            removed = Some(name);
        }
    }
    tx.commit()?;
    Ok(removed)
}

pub fn get_file(conn: &Connection, viewer: &Viewer, public_id: &str) -> rusqlite::Result<Option<StoredFile>> {
    let f = visibility::files(viewer, "f");
    let mut params = vec![Value::Text(public_id.to_string())];
    params.extend(f.params);
    conn.query_row(
        &format!("SELECT f.public_id, f.name, f.content_type FROM files f WHERE f.public_id = ? AND {}", f.sql),
        params_from_iter(params),
        |r| Ok(StoredFile { public_id: r.get(0)?, name: r.get(1)?, content_type: r.get(2)? }),
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markdown::analyze;

    fn post(conn: &mut Connection, author: &str, body: &str, vis: Visibility) -> String {
        create_post(conn, author, body, vis, &analyze(body)).unwrap()
    }

    fn max() -> Viewer {
        Viewer::User("max".into())
    }
    fn anna() -> Viewer {
        Viewer::User("anna".into())
    }

    #[test]
    fn visibility_of_posts() {
        let mut c = open_in_memory().unwrap();
        let public = post(&mut c, "max", "hello #sauna", Visibility::Public);
        let private = post(&mut c, "max", "secret #sauna #diary", Visibility::Private);

        assert!(get_post(&c, &max(), &private).unwrap().is_some());
        assert!(get_post(&c, &anna(), &private).unwrap().is_none());
        assert!(get_post(&c, &Viewer::Anonymous, &private).unwrap().is_none());
        assert!(get_post(&c, &Viewer::Anonymous, &public).unwrap().is_some());

        let list = |v: &Viewer, s: &Scope| list_posts(&c, v, s, None, 50).unwrap().len();
        assert_eq!(list(&max(), &Scope::Own), 2);
        assert_eq!(list(&anna(), &Scope::Own), 0);
        assert_eq!(list(&Viewer::Anonymous, &Scope::Own), 0);
        assert_eq!(list(&max(), &Scope::Public), 1);
        assert_eq!(list(&anna(), &Scope::Author("max")), 1);
        assert_eq!(list(&anna(), &Scope::Tag("sauna")), 1);
        assert_eq!(list(&max(), &Scope::Tag("sauna")), 2);
        assert_eq!(list(&anna(), &Scope::Tag("diary")), 0);

        let counts = count_posts(&c, &max(), &Scope::Tag("sauna")).unwrap();
        assert_eq!((counts.total, counts.public), (2, 1));
        assert_eq!(count_posts(&c, &anna(), &Scope::Tag("sauna")).unwrap().total, 1);

        assert_eq!(tags_with_prefix(&c, &max(), "", 10).unwrap().len(), 2);
        assert_eq!(tags_with_prefix(&c, &anna(), "", 10).unwrap(), vec![("sauna".to_string(), 1)]);
        assert!(tags_with_prefix(&c, &Viewer::Anonymous, "di", 10).unwrap().is_empty());
    }

    #[test]
    fn writes_need_ownership() {
        let mut c = open_in_memory().unwrap();
        let id = post(&mut c, "max", "mine", Visibility::Private);
        assert!(!set_visibility(&c, &anna(), &id, Visibility::Public).unwrap());
        assert!(!update_post(&mut c, &anna(), &id, "x", Visibility::Public, &analyze("x")).unwrap());
        assert!(delete_post(&mut c, &anna(), &id).unwrap().is_none());
        assert!(set_visibility(&c, &max(), &id, Visibility::Public).unwrap());
        assert!(get_post(&c, &anna(), &id).unwrap().is_some());
    }

    #[test]
    fn files_follow_their_post() {
        let mut c = open_in_memory().unwrap();
        let file = create_file(&c, "max", "jpg", "image/jpeg").unwrap();
        // Unattached: owner only.
        assert!(get_file(&c, &max(), &file).unwrap().is_some());
        assert!(get_file(&c, &anna(), &file).unwrap().is_none());

        let body = format!("lake ![](/files/{file})");
        let id = post(&mut c, "max", &body, Visibility::Public);
        assert!(get_file(&c, &Viewer::Anonymous, &file).unwrap().is_some());

        set_visibility(&c, &max(), &id, Visibility::Private).unwrap();
        assert!(get_file(&c, &Viewer::Anonymous, &file).unwrap().is_none());
        assert!(get_file(&c, &anna(), &file).unwrap().is_none());
        assert!(get_file(&c, &max(), &file).unwrap().is_some());

        // Someone else's public post can't publish max's file.
        post(&mut c, "anna", &format!("stolen ![](/files/{file})"), Visibility::Public);
        assert!(get_file(&c, &Viewer::Anonymous, &file).unwrap().is_none());

        let removed = delete_post(&mut c, &max(), &id).unwrap().unwrap();
        assert_eq!(removed, vec![format!("{file}.jpg")]);
        assert!(get_file(&c, &max(), &file).unwrap().is_none());
    }

    #[test]
    fn paging() {
        let mut c = open_in_memory().unwrap();
        for i in 0..5 {
            post(&mut c, "max", &format!("post {i}"), Visibility::Public);
        }
        let first = list_posts(&c, &max(), &Scope::Own, None, 3).unwrap();
        let rest = list_posts(&c, &max(), &Scope::Own, Some(Cursor::of(first.last().unwrap())), 3).unwrap();
        assert_eq!(first.len() + rest.len(), 5);
        assert_eq!(rest.last().unwrap().body, "post 0");
        assert_eq!(Cursor::decode(&Cursor::of(&first[0]).encode()), Some(Cursor::of(&first[0])));
    }
}
