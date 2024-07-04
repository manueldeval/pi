use std::env;

use axum::{response::IntoResponse, routing::get, Json, Router};
use tokio::signal;

pub async fn status_handler() -> impl IntoResponse {

    let json_response = serde_json::json!({
        "status": "success",
        "message": "Pi is ok."
    });
    Json(json_response)
}

#[tokio::main]
pub async fn main() {
    let key = "PORT";
    let port = env::var(key).unwrap_or("3000".to_string());

    println!("Server started successfully on port {}",port);
    let route = Router::new().route("/api/status", get(status_handler));
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}",port)).await.unwrap();
    axum::serve(listener, route).with_graceful_shutdown(shutdown_signal()).await.unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
