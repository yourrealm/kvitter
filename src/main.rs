use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    process::ExitCode,
    time::Duration,
};

use kvitter::{AppState, app, config::Config, db, import};

const USAGE: &str = "usage: kvitter [serve [--dev-user NAME]] | kvitter healthcheck\n       kvitter import-memos MEMOS_DB --as USER";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        [] | ["serve"] => serve(None).await,
        ["serve", "--dev-user", user] => serve(Some(user.to_string())).await,
        ["healthcheck"] => healthcheck(),
        ["import-memos", source, "--as", user] => import_memos(source, user),
        _ => Err(USAGE.to_string()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("kvitter: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn serve(dev_user: Option<String>) -> Result<(), String> {
    let config = Config::from_env(dev_user)?;
    if let Some(user) = &config.dev_user {
        eprintln!("kvitter: DEV MODE, every request is signed in as {user:?} except on public.* hosts");
    } else if config.proof.is_none() {
        eprintln!("kvitter: KVITTER_PROOF is not set, so nobody can sign in");
    }
    for dir in [&config.data_dir, &config.uploads_dir] {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let conn = db::open(&config.data_dir.join("kvitter.db")).map_err(|e| format!("database: {e}"))?;
    let addr = SocketAddr::from(([0, 0, 0, 0], config.port));
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("{addr}: {e}"))?;
    eprintln!("kvitter: listening on {addr}");
    axum::serve(listener, app(AppState::new(config, conn)))
        .with_graceful_shutdown(shutdown())
        .await
        .map_err(|e| e.to_string())
}

/// Safe to run next to a live server (SQLite WAL), and to re-run: imported memos are skipped.
fn import_memos(source: &str, user: &str) -> Result<(), String> {
    let config = Config::from_env(None)?;
    std::fs::create_dir_all(&config.uploads_dir).map_err(|e| format!("{}: {e}", config.uploads_dir.display()))?;
    let mut conn = db::open(&config.data_dir.join("kvitter.db")).map_err(|e| format!("database: {e}"))?;
    let s = import::memos(&mut conn, &config.uploads_dir, std::path::Path::new(source), user)?;
    for problem in &s.problems {
        eprintln!("kvitter: {problem}");
    }
    eprintln!(
        "kvitter: imported {} public and {} private posts as {user:?}, with {} images; {} already imported, {} empty",
        s.public, s.private, s.images, s.skipped, s.empty
    );
    Ok(())
}

async fn shutdown() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM handler");
    tokio::select! {
        _ = ctrl_c => {}
        _ = term.recv() => {}
    }
}

/// Exit 0 when the server answers `/healthz`. Used by the container healthcheck, where the
/// distroless image has no curl or shell.
fn healthcheck() -> Result<(), String> {
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000);
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).map_err(|e| e.to_string())?;
    stream
        .write_all(b"GET /healthz HTTP/1.0\r\nHost: localhost\r\n\r\n")
        .map_err(|e| e.to_string())?;
    let mut response = String::new();
    stream.read_to_string(&mut response).map_err(|e| e.to_string())?;
    if response.starts_with("HTTP/1.0 200") || response.starts_with("HTTP/1.1 200") {
        Ok(())
    } else {
        Err(format!("unhealthy: {}", response.lines().next().unwrap_or("no response")))
    }
}
