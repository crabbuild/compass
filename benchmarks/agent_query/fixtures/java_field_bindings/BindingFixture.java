package bindings;
import static java.lang.Integer.MAX_VALUE;

// UTF-16 and UTF-8 offsets differ after λ and 🧭.
public class BindingFixture extends Base {
    int left, right;
    static final int LIMIT = 3;
    String label = "λ🧭";
    int initial = left; // expect left; owner initial
    { right = left; } // expect right,left; owner BindingFixture
    int read(int seed) { return left; } // expect left; overload 1
    int read(String seed) { return right; } // expect right; overload 2
    int compound() { left += right; return left; } // expect left,right,left
    int constant() { return LIMIT; } // expect LIMIT despite bytecode folding
    int external() { return MAX_VALUE; } // external field, no source declaration
    int commented(BindingFixture self) { return self./* comment */left; } // expect left
    int \u0078;
    int escaped() { return \u0078; } // expect x with unsupported raw spelling anchor
    java.util.function.IntSupplier lambda() { return () -> left; } // expect left; owner lambda
    int hidden() { return super.left + left; } // expect Base.left,left
    Object anonymous() {
        return new Object() {
            int left;
            int get() { return left; } // expect anonymous left; owner get
        };
    }
    int local() {
        class Local {
            int left;
            int get() { return left; } // expect local left; owner get
        }
        return new Local().get();
    }
    enum Token { A, B }
    Token token() { return Token.A; } // expect enum constant A
    int intrinsic(int[] values) { return values.length + BindingFixture.class.getName().length(); } // two intrinsics
}
class Base { int left; }
