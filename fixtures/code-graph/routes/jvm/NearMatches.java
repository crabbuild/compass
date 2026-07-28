package example;

public class NearMatches {
    @GetMapping("/not-spring")
    public String get() { return "no"; }
}

@interface GetMapping {
    String value();
}
