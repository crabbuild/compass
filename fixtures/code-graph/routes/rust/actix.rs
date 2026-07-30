use actix_web::{web, App};

async fn list_users() {}

fn app() -> App {
    App::new().route("/users", web::get().to(list_users))
}
