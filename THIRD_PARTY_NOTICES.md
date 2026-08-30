# Third-party notices

## Optional SurrealDB 3.2.4 integration

The non-default `mem`, `surrealkv`, and `rocksdb` features of
`compass-graphdb-surreal` resolve SurrealDB 3.2.4 and its core components.
Those components are licensed under Business Source License 1.1 before
conversion, with SurrealDB Ltd. as licensor, a Database Service restriction,
Change Date 2030-01-01, and Apache License 2.0 as the Change License.

Surreal-enabled artifacts must preserve the applicable exact license and
notices and comply with its redistribution terms. The reviewed tagged license
is retained at
`scripts/fixtures/surreal-persistent-probes/SURREALDB-3.2.4-LICENSE.txt`; its
SHA-256 is
`98a94ac615f88370865016487b436fa404560910bd329794ed7502277a94b805`.
The default Compass binary does not link these components.

## Local document OCR and PDF rendering

Compass links OAR-OCR 0.9.2 and its `ort`/ONNX Runtime integration for optional
local OCR. OAR-OCR is Apache-2.0; the Rust `ort` crates are MIT or Apache-2.0.
Compass also links Hayro 0.7.1 for pure-Rust PDF rendering under MIT or
Apache-2.0. OAR-OCR's text-region geometry stack links clipper2-rust 1.1.0
under BSL-1.0. The corresponding license texts are covered by `LICENSE-MIT`,
`LICENSE-APACHE`, and `LICENSE-BOOST` in release bundles.

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
