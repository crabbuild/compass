use axum::{Router, routing::{get, post}};

async fn show_user() {}
async fn create_user() {}

fn router() -> Router {
    Router::new()
        .route("/users/:id", get(show_user))
        .route("/users", post(create_user))
}
