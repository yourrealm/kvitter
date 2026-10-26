use std::path::PathBuf;

use chrono_tz::Tz;

#[derive(Clone, Debug)]
pub struct Config {
    /// Public domain origin without a trailing slash, or empty when none is set.
    pub public_url: String,
    /// Shared secret Realm sends as `X-Kvitter-Proof`. `None` means no request is ever signed in.
    pub proof: Option<String>,
    pub data_dir: PathBuf,
    pub uploads_dir: PathBuf,
    pub port: u16,
    pub tz: Tz,
    /// Local development only: sign every request in as this user, except on `public.*` hosts.
    pub dev_user: Option<String>,
}

impl Config {
    pub fn from_env(dev_user: Option<String>) -> Result<Self, String> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        let tz = match var("KVITTER_TZ") {
            Some(name) => name.parse().map_err(|_| format!("KVITTER_TZ: unknown time zone {name:?}"))?,
            None => Tz::UTC,
        };
        Ok(Config {
            public_url: var("PUBLIC_URL").map(|u| u.trim_end_matches('/').to_string()).unwrap_or_default(),
            proof: var("KVITTER_PROOF"),
            data_dir: var("DATA_DIR").unwrap_or_else(|| "/data".into()).into(),
            uploads_dir: var("UPLOADS_DIR").unwrap_or_else(|| "/uploads".into()).into(),
            port: match var("PORT") {
                Some(p) => p.parse().map_err(|_| format!("PORT: not a port number: {p:?}"))?,
                None => 3000,
            },
            tz,
            dev_user,
        })
    }
}
