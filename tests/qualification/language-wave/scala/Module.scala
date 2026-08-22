package wave

trait Store { def save(value: String): Unit }
final case class User(name: String)
final class UserStore extends Store {
  override def save(value: String): Unit = println(value)
  def route(): Unit = save("users")
}
object UserStore { def apply(): UserStore = new UserStore() }
given ordering: Ordering[User] with
  def compare(left: User, right: User): Int = left.name.compareTo(right.name)
extension (store: UserStore) def repeated(): Unit = { store.save("a"); store.save("b") }
