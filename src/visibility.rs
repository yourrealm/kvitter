//! The one visibility filter. Every query that returns posts, tags, counts or files builds its
//! WHERE clause from here; no query writes its own visibility condition.

use rusqlite::types::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Viewer {
    Anonymous,
    User(String),
}

impl Viewer {
    pub fn user(&self) -> Option<&str> {
        match self {
            Viewer::User(u) => Some(u),
            Viewer::Anonymous => None,
        }
    }

    pub fn is_signed_in(&self) -> bool {
        self.user().is_some()
    }

    pub fn owns(&self, author: &str) -> bool {
        self.user() == Some(author)
    }
}

/// A SQL condition plus its parameters.
pub struct Filter {
    pub sql: String,
    pub params: Vec<Value>,
}

/// Posts (aliased as `alias`) the viewer may see: public ones, plus their own.
pub fn posts(viewer: &Viewer, alias: &str) -> Filter {
    match viewer {
        Viewer::Anonymous => Filter { sql: format!("{alias}.visibility = 'public'"), params: vec![] },
        Viewer::User(u) => Filter {
            sql: format!("({alias}.visibility = 'public' OR {alias}.author = ?)"),
            params: vec![Value::Text(u.clone())],
        },
    }
}

/// Files (aliased as `alias`) the viewer may fetch: their own uploads, or files attached to a
/// post the viewer can see. Attachment only ever links a file to a post by the same owner.
pub fn files(viewer: &Viewer, alias: &str) -> Filter {
    let visible = posts(viewer, "vp");
    // Attached to a visible post, or the avatar of someone with a visible post: an avatar
    // shows no more than the author's name next to their posts already does.
    let attached = format!(
        "(EXISTS (SELECT 1 FROM post_files vpf JOIN posts vp ON vp.id = vpf.post_id \
         WHERE vpf.file_id = {alias}.id AND {v}) \
         OR EXISTS (SELECT 1 FROM avatars va JOIN posts vp ON vp.author = va.user \
         WHERE va.file_id = {alias}.id AND {v}))",
        v = visible.sql
    );
    match viewer {
        Viewer::Anonymous => Filter { sql: attached, params: [visible.params.clone(), visible.params].concat() },
        Viewer::User(u) => {
            let mut params = vec![Value::Text(u.clone())];
            params.extend(visible.params.clone());
            params.extend(visible.params);
            Filter { sql: format!("({alias}.owner = ? OR {attached})"), params }
        }
    }
}
