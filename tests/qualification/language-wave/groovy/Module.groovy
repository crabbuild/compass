package wave

interface Store { void save(String value) }
class UserStore implements Store {
    String value
    UserStore() {}
    void save(String value) { this.value = value }
    void route() { save('users') }
}
trait Audited { void audit() {} }
class Specification extends spock.lang.Specification {
    def "stores users"() { expect: new UserStore().save('ok') }
}
