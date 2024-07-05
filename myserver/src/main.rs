use std::env;

use axum::{ body::{self, Body}, extract::Path, http::{header, HeaderValue, Response, StatusCode}, response::{IntoResponse, Redirect}, routing::get, Json, Router};
use include_dir::{Dir,include_dir};
use tokio::signal;

static STATIC_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/static");

pub async fn status_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "status": "success",
        "message": "Pi ❤️ Rust!"
    }))
}

pub async fn health_handler() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn static_path(Path(path): Path<String>) -> impl IntoResponse {
    let path = path.trim_start_matches('/');
    let mime_type = mime_guess::from_path(path).first_or_text_plain();

    match STATIC_DIR.get_file(path) {
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::empty())
            .unwrap(),
        Some(file) => Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_str(mime_type.as_ref()).unwrap(),
            )
            .body(Body::from(file.contents()))
            .unwrap(),
    }
}
// .body(file.contents())

#[tokio::main]
pub async fn main() {
    let port = env::var("PORT")
        .map(|s| s.parse::<u16>().expect(format!("Unable to parse the env var PORT: {}",s).as_str()))
        .unwrap_or(3000);

    println!("Server started successfully on port {}",port);
    
    let route = Router::new()
        .route("/static/*path", get(static_path))
        .route("/", get(|| async { Redirect::permanent("/static/love.png") }))
        .route("/api/status", get(status_handler))
        .route("/health", get(health_handler));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}",port)).await.unwrap();
    
    axum::serve(listener, route).with_graceful_shutdown(shutdown_signal()).await.unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async { signal::ctrl_c().await.expect("failed to install Ctrl+C handler"); };

    #[cfg(unix)]
    let terminate = async { signal::unix::signal(signal::unix::SignalKind::terminate()).expect("failed to install signal handler").recv().await; };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
