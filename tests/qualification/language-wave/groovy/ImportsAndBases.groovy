package wave

import java.util.List
import static java.util.Collections.emptyList as empty

interface Contract {}
class Base {}
class Child extends Base implements Contract, List<String> {
    List<String> values = empty()
    Base make() { new Base() }
}
