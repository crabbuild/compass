package audit;

import java.util.function.IntSupplier;
import java.util.function.IntUnaryOperator;

// Compiler-checkable development fixture; no product runtime dependency on Java.
public class ScopeFixture extends Base {
    int value; // field scope_value
    Cell slot; // field scope_slot
    Cell[] items; // field scope_items
    RuntimeException failure; // field scope_failure
    Token resource; // field scope_resource
    Object pattern; // field scope_pattern
    static int staticValue; // field scope_static

    ScopeFixture(int value) {
        this.value = value; // case constructor: scope_value
    }
    int explicit() {
        return this.value; // case explicit_this: scope_value
    }
    int implicit() {
        return value; // case implicit_this: scope_value
    }
    int repeated() {
        return value + this.value; // case repeated: scope_value,scope_value
    }
    int parameter(int value) {
        return value; // case parameter_shadow: -
    }
    void block() {
        sink(value); // case block_before: scope_value
        {
            int value = 7;
            sink(value); // case block_shadow: -
        }
        sink(value); // case block_after: scope_value
    }
    int beforeLocal() {
        sink(value); // case before_local: scope_value
        int value = 3;
        return value; // case after_local: -
    }
    void loop() {
        for (int value = 0; value < 1; value++) {
            sink(value); // case for_shadow: -
        }
        sink(value); // case for_after: scope_value
    }
    int enhanced() {
        for (Cell slot : items) { // case enhanced_iterable: scope_items
            sink(slot.value); // case enhanced_binding: cell_value
        }
        return slot.value; // case enhanced_after: scope_slot,cell_value
    }
    void lambdas() {
        IntUnaryOperator a = value -> value + 1; // case lambda_shadow: -
        IntSupplier b = () -> this.value; // case lambda_this: scope_value
        IntSupplier c = () -> value; // case lambda_implicit: scope_value
        sink(a.applyAsInt(b.getAsInt()) + c.getAsInt());
    }
    void catches() {
        try { throw new RuntimeException(); }
        catch (RuntimeException failure) {
            sink(failure == null ? 0 : 1); // case catch_shadow: -
        }
        sink(failure == null ? 0 : 1); // case catch_after: scope_failure
    }
    void resources() {
        try (Token resource = new Token()) {
            sink(resource.value); // case resource_binding: token_value
        } catch (RuntimeException e) {
            sink(resource.value); // case resource_catch: scope_resource,token_value
        }
        sink(resource.value); // case resource_after: scope_resource,token_value
    }
    int pattern(Object input) {
        if (input instanceof Cell slot) {
            return slot.value; // case pattern_true: cell_value
        }
        return slot.value; // case pattern_after: scope_slot,cell_value
    }
    int negatedPattern() {
        if (!(pattern instanceof Cell slot)) return 0; // case pattern_input: scope_pattern
        return slot.value; // case pattern_flow: cell_value
    }
    int typed(Cell cell) {
        return cell.value; // case typed_parameter: cell_value
    }
    int cast(Object cell) {
        return ((Cell) cell).value; // case cast_receiver: cell_value
    }
    int chain() {
        return slot.value; // case field_chain: scope_slot,cell_value
    }
    int indexed() {
        return items[0].value; // case array_receiver: scope_items,cell_value
    }
    int parent() {
        return super.value; // case super_field: base_value
    }
    int staticBinding(Cell cell, Shadow shadow) {
        return cell.value; // case declared_receiver: cell_value
    }
    int subtype(Shadow cell) {
        return cell.value; // case hiding_receiver: shadow_value
    }
    <T extends Cell> int generic(T cell) {
        return cell.value; // case generic_bound: cell_value
    }
    static int statics() {
        return staticValue + ScopeFixture.staticValue; // case static_fields: scope_static,scope_static
    }
    int value() { return 1; }
    int methodSelector() {
        return value(); // case method_not_field: -
    }
    void declarators() {
        int first = value, value = 2; // case declarator_order: scope_value
        sink(first + value); // case declarator_shadow: -
    }
    class Inner {
        int value; // field inner_value
        int own() {
            return value; // case inner_own: inner_value
        }
        int outer() {
            return ScopeFixture.this.value; // case qualified_this: scope_value
        }
    }
    class Inherited extends Base {
        int inherited() {
            return value; // case inherited_beats_outer: base_value
        }
    }
    IntSupplier anonymous() {
        return new IntSupplier() {
            int value; // field anonymous_value
            public int getAsInt() {
                return this.value + ScopeFixture.this.value; // case anonymous_this: anonymous_value,scope_value
            }
        };
    }
    int local() {
        class Local {
            int value; // field local_value
            int get() {
                return this.value; // case local_class_this: local_value
            }
        }
        return new Local().get();
    }
    static void sink(int value) {}
}
class Base {
    int value; // field base_value
}
class Cell {
    int value; // field cell_value
}
class Shadow extends Cell {
    int value; // field shadow_value
}
class Token implements AutoCloseable {
    int value; // field token_value
    public void close() {}
}
