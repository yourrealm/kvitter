//! `kvitter import-memos`: copies posts from a usememos database. It reports counts and memo
//! ids only, never content, since most memos are private.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};

use crate::{
    db::{self, Visibility},
    images,
    markdown::analyze,
    views::MAX_POST_CHARS,
};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub public: usize,
    pub private: usize,
    pub images: usize,
    /// Already imported by an earlier run.
    pub skipped: usize,
    /// No text and no images.
    pub empty: usize,
    /// Memo ids with what went wrong, for the ones not imported or imported without an image.
    pub problems: Vec<String>,
}

struct Memo {
    id: i64,
    uid: String,
    content: String,
    public: bool,
    created: i64,
    updated: i64,
}

/// Only memos that are public in memos and not archived become public; everything else,
/// including PROTECTED (visible to signed-in memos users), becomes private.
pub fn memos(conn: &mut Connection, uploads_dir: &Path, source: &Path, author: &str) -> Result<Summary, String> {
    let memos = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("{}: {e}", source.display()))?;
    let all = read_memos(&memos).map_err(|e| format!("reading memos: {e}"))?;
    let mut summary = Summary::default();
    for memo in all {
        let source_key = format!("memos:{}", memo.uid);
        if db::is_imported(conn, &source_key).map_err(|e| e.to_string())? {
            summary.skipped += 1;
            continue;
        }
        let mut body = memo.content.trim().to_string();
        for attachment in attachments(&memos, memo.id).map_err(|e| format!("memo {}: {e}", memo.uid))? {
            let Some(bytes) = attachment.blob else {
                summary.problems.push(format!("memo {}: an attachment is stored outside the database", memo.uid));
                continue;
            };
            let processed = match images::process(&bytes) {
                Ok(p) => p,
                Err(e) => {
                    summary.problems.push(format!("memo {}: {} attachment skipped: {e}", memo.uid, attachment.kind));
                    continue;
                }
            };
            let id = db::create_file(conn, author, processed.ext, processed.content_type).map_err(|e| e.to_string())?;
            let path = uploads_dir.join(format!("{id}.{}", processed.ext));
            std::fs::write(&path, &processed.bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            if !body.is_empty() {
                body.push_str("\n\n");
            }
            body.push_str(&format!("![](/files/{id})"));
            summary.images += 1;
        }
        if body.is_empty() {
            summary.empty += 1;
            continue;
        }
        if body.chars().count() > MAX_POST_CHARS {
            summary.problems.push(format!("memo {}: longer than {MAX_POST_CHARS} characters, skipped", memo.uid));
            continue;
        }
        let visibility = if memo.public { Visibility::Public } else { Visibility::Private };
        db::import_post(conn, &source_key, author, &body, visibility, (memo.created, memo.updated), &analyze(&body))
            .map_err(|e| format!("memo {}: {e}", memo.uid))?;
        match visibility {
            Visibility::Public => summary.public += 1,
            Visibility::Private => summary.private += 1,
        }
    }
    Ok(summary)
}

fn read_memos(memos: &Connection) -> rusqlite::Result<Vec<Memo>> {
    let mut stmt = memos.prepare(
        "SELECT id, uid, content, visibility = 'PUBLIC' AND row_status = 'NORMAL', created_ts, updated_ts
         FROM memo ORDER BY created_ts, id",
    )?;
    stmt.query_map([], |r| {
        Ok(Memo {
            id: r.get(0)?,
            uid: r.get(1)?,
            content: r.get(2)?,
            public: r.get(3)?,
            created: r.get(4)?,
            updated: r.get(5)?,
        })
    })?
    .collect()
}

struct Attachment {
    blob: Option<Vec<u8>>,
    kind: String,
}

fn attachments(memos: &Connection, memo_id: i64) -> rusqlite::Result<Vec<Attachment>> {
    let mut stmt = memos.prepare("SELECT blob, type FROM attachment WHERE memo_id = ? ORDER BY id")?;
    stmt.query_map(params![memo_id], |r| Ok(Attachment { blob: r.get(0)?, kind: r.get(1)? }))?.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visibility::Viewer;

    fn fake_memos(path: &Path) {
        let c = Connection::open(path).unwrap();
        c.execute_batch(
            "CREATE TABLE memo (id INTEGER PRIMARY KEY, uid TEXT, content TEXT, visibility TEXT,
                                row_status TEXT, created_ts INTEGER, updated_ts INTEGER);
             CREATE TABLE attachment (id INTEGER PRIMARY KEY, memo_id INTEGER, blob BLOB, type TEXT);
             INSERT INTO memo VALUES
               (1, 'pub', 'hello #sauna', 'PUBLIC', 'NORMAL', 1000, 1000),
               (2, 'priv', 'secret', 'PRIVATE', 'NORMAL', 2000, 2500),
               (3, 'prot', 'members', 'PROTECTED', 'NORMAL', 3000, 3000),
               (4, 'arch', 'old', 'PUBLIC', 'ARCHIVED', 4000, 4000),
               (5, 'img', '', 'PUBLIC', 'NORMAL', 5000, 5000),
               (6, 'blank', '  ', 'PRIVATE', 'NORMAL', 6000, 6000);",
        )
        .unwrap();
        let mut png = Vec::new();
        image::RgbImage::new(4, 4).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        c.execute("INSERT INTO attachment (memo_id, blob, type) VALUES (5, ?, 'image/png')", [png]).unwrap();
        c.execute("INSERT INTO attachment (memo_id, blob, type) VALUES (5, x'00', 'application/pdf')", []).unwrap();
    }

    #[test]
    fn imports_once_and_keeps_private_private() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("memos.db");
        fake_memos(&source);
        let mut conn = db::open_in_memory().unwrap();

        let s = memos(&mut conn, dir.path(), &source, "max").unwrap();
        assert_eq!((s.public, s.private, s.images, s.empty, s.skipped), (2, 3, 1, 1, 0));
        assert_eq!(s.problems.len(), 1, "the pdf");

        let public = db::list_posts(&conn, &Viewer::Anonymous, &db::Scope::Public, None, 10).unwrap();
        let bodies: Vec<_> = public.iter().map(|p| p.body.as_str()).collect();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[1], "hello #sauna");
        assert!(bodies[0].starts_with("![](/files/"));
        assert_eq!(public[1].created_at, 1000);
        // The image follows its public post.
        let file = &bodies[0][11..19];
        assert!(db::get_file(&conn, &Viewer::Anonymous, file).unwrap().is_some());

        let again = memos(&mut conn, dir.path(), &source, "max").unwrap();
        assert_eq!((again.public, again.private, again.skipped), (0, 0, 5));
    }
}
