package routes

class UserService {
    String load() { "ok" }
    void run() { load(); load() }
}

class UserSpec {
    UserService service = new UserService()
    def "loads users"() {
        service.load()
        service.load()
    }
}
