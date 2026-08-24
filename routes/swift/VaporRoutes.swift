import Vapor

func listUsers(_ request: Request) async throws -> String { "users" }
func createUser(_ request: Request) async throws -> String { "created" }

func routes(_ app: Application) throws {
    let api = app.grouped("api")
    api.get("users", use: listUsers)
    api.post("users", use: createUser)
    api.get("health") { request in "ok" }
}
