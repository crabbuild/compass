package audit;

public final class VarargsDemo {
    static String join(String... values) { return "text"; }
    static String join(int... values) { return "numbers"; }
    static String join(boolean flag) { return "flag"; }
    static String prefixed(String prefix, String... values) { return prefix; }
    static String arrays(String[] values, int[] counts) { return "arrays"; }
    static String text() { return join("a", "b"); }
    static String numbers() { return join(1, 2); }
    static String flag() { return join(true); }
    static String explicitArray() { return join(new String[] {"a", "b"}); }
    static String mixed() { return prefixed("p", "a", "b"); }
}
