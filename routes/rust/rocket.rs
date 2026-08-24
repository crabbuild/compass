use rocket::get;

#[get("/users/<id>")]
fn show_user(id: u64) {}
