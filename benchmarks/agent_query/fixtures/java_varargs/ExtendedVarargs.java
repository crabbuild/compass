package audit;

public class ExtendedVarargs {
    static void strings(String... values) {}
    static void integers(int... values) {}
    static void shape(String value) {}
    static void shape(String[] value) {}
    static void shape(String[][] value) {}
    static void choose(Object... values) {}
    static void choose(String... values) {}
    static void phase(Object value) {}
    static void phase(String... values) {}
    static void widen(long value) {}
    static void widen(Integer value) {}
    static void loose(Integer value) {}
    static void loose(int... values) {}
    static void primitive(long... values) {}
    static void primitive(int... values) {}
    static void prefixed(String prefix, int... values) {}

    static void empty() { strings(); }
    static void mostSpecificEmpty() { choose(); }
    static void mostSpecificMany() { choose("a", "b"); }
    static void nullArray() { choose((String[]) null); }
    static void fixedBeforeExpanded() { phase("a"); }
    static void wideningBeforeBoxing() { widen(1); }
    static void boxingBeforeExpanded() { loose(1); }
    static void primitiveSpecific() { primitive(1, 2); }
    static void prefixEmpty() { prefixed("a"); }
    static void arrayArgument() { shape(new String[1]); }
    static void matrixArgument() { shape(new String[1][]); }
    static void matrixInitializer() { shape(new String[][] {{"a"}}); }
    static void spreadForward(final String... values) { shape(values); }
    static void trailingDimensions(String values[]) { shape(values); }
    static void repeated() { strings("a"); strings("b"); }
    static void unknown(String value) { choose(value.toString()); }

    void receiver(ExtendedVarargs this, String... values) { shape(values); }
    static void modified(@Deprecated final String... values) { shape(values); }
    static void arrays(String[]... values) { shape(values); }
    static void generic(java.util.List<String>... values) {}
}
