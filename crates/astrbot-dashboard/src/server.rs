use axum::{Router, routing::get};
use std::net::SocketAddr;

pub async fn start_server() {
    let app = Router::new()
        .route("/api/health", get(|| async { "OK" }));

    let addr = SocketAddr::from(([127, 0, 0, 1], 6185));
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
