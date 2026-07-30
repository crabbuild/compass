fn get<T>(handler: T) -> T { handler }
fn show() {}

fn configure() {
    fake.route("/not-a-route", get(show));
}
