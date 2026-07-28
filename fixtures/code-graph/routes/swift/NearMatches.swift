struct Fake {
    func get(_ path: String, use handler: () -> Void) {}
}

func handler() {}

func configure(_ fake: Fake) {
    fake.get("not-a-route", use: handler)
}
