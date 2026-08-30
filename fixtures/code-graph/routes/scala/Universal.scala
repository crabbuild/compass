package routes

trait Store { def save(value: String): Unit }

class UserService extends Store {
  def save(value: String): Unit = ()
  def run(): Unit = { save("a"); save("b") }
}
