# Third-party notices

## Local document OCR and PDF rendering

Compass links OAR-OCR 0.9.2 and its `ort`/ONNX Runtime integration for optional
local OCR. OAR-OCR is Apache-2.0; the Rust `ort` crates are MIT or Apache-2.0.
Compass also links Hayro 0.7.1 for pure-Rust PDF rendering under MIT or
Apache-2.0. The corresponding license texts are covered by `LICENSE-MIT` and
`LICENSE-APACHE` in release bundles.

The separately installed `pp-ocrv6-small` and `pp-ocrv6-medium` model files
come from the immutable GreatV/OAR-OCR `v0.7.0` release and originate from the
PaddleOCR project. They are distributed under Apache-2.0 and are not embedded
in Compass release archives. Compass verifies their exact sizes and SHA-256
digests before use.

## openCypher Technology Compatibility Kit feature files

Compass includes selected unmodified Gherkin feature files from the
[openCypher project](https://github.com/opencypher/openCypher), tag `2024.3`,
commit `677cbafabb8c3c5eed458fd3b1ec0daec8d67d23`.

Copyright (c) Neo4j Sweden AB and the openCypher contributors. Licensed under
the Apache License, Version 2.0. The original files retain their copyright,
license, attribution, and trademark notices. A copy of the license is at
`tests/opencypher-tck/LICENSE`.

This product is an independent implementation. It is not approved by or
affiliated with Neo4j or the openCypher Implementers Group. “Cypher” is a
registered trademark of Neo4j, Inc.

## Lucide icons

The Compass VS Code extension uses and adapts the Lucide Compass icon.
Copyright Lucide Contributors. Licensed under the ISC License.

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the copyright
notice and this permission notice appear in all copies.
