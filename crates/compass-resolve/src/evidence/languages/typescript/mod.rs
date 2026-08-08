//! TypeScript and JavaScript module, type, and member policy.

use super::super::*;

mod members;
mod modules;
mod overloads;
mod types;

use overloads::*;
use types::*;
pub(in crate::evidence) use types::{
    typescript_declaration_basic_allowed, typescript_declaration_basic_allowed_with_type_owner,
};
