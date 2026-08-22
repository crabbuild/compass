import Vapor

public protocol Store { func save(_ value: String) async throws }
public struct UserStore: Store {
    public init() {}
    public func save(_ value: String) async throws { _ = value.count }
}
public extension UserStore {
    func route(_ app: Application) {
        app.get("users", use: listUsers)
    }
    private func listUsers(_ request: Request) async throws -> String { "ok" }
}

struct AmbiguousA { func same() {} }
struct AmbiguousB { func same() {} }
func repeated(_ store: UserStore) {
    _ = store.save
    _ = store.save
}
