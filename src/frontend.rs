use crate::AppState;
use axum::{response::Html, routing::get, Router};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .route("/app", get(index))
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../frontend/index.html"))
}
