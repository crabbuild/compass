#![allow(clippy::expect_used, clippy::panic)]

//! Universal resolver integration characterization, grouped by ownership.

include!("universal_resolution/core.rs");
include!("universal_resolution/rust.rs");
include!("universal_resolution/python.rs");
include!("universal_resolution/go.rs");
include!("universal_resolution/typescript.rs");
include!("universal_resolution/javascript.rs");
