# Independent language providers

These four helpers are qualification-only source parsers. They receive the
already validated file list from the Python boundary, never execute repository
code, and emit the versioned source-oracle contract consumed by the audit
harness. They are not Compass runtime dependencies.

The current release-candidate toolchains live outside the checkout under
`/Volumes/Workspace/crabbuild-target/compass-main/providers`:

- `swift_oracle.swift`: Swift 6.3.3 with SwiftSyntax 603.0.0. Build it from a
  small SwiftPM executable target that depends on the pinned SwiftSyntax
  checkout and copies `swift_oracle.swift` to `Sources/CompassSwiftOracle/main.swift`.
- `dart_oracle.dart`: Dart SDK 3.13.1 with `analyzer` 8.4.0. The external
  package uses `dart pub get` followed by `dart compile exe`.
- `scala_oracle.scala`: Scala CLI 1.9.1, Scala 3.7.3, scala.meta 4.13.10, and
  ujson 4.1.0. The `//> using` directives pin the source dependencies; package
  it as `compass-scala-oracle`.
- `groovy_oracle.java`: Apache Groovy 4.0.27. Compile it with `javac` against
  the pinned `groovy-4.0.27.jar` and launch the resulting class with that jar
  on the class path.

Provider binaries and dependency caches stay on the mounted target volume and
must never be committed to this repository. The four `*_source_oracle.py`
wrappers fail closed to `parserAvailable: false` when the corresponding
executable is absent; such fallback output is intentionally rejected by the
quality-audit evaluator.
