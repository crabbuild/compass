//! Immutable secondary indexes over validated evidence facts.

use super::*;

mod builder;

pub(super) struct ResolutionIndexes {
    pub(super) by_qualified: AHashMap<(String, String), Vec<DeclarationSlot>>,
    pub(super) by_module_name: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    pub(super) by_scope_name: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    pub(super) by_source_directory_name: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    /// TypeScript/JavaScript source modules are indexed by the normalized
    /// repository-relative module path rather than by a basename. This keeps
    /// relative imports project-aware without allowing terminal-name lookup
    /// to select a same-spelled declaration from another directory.
    pub(super) typescript_modules: TypeScriptModuleIndex,
    /// Declaration slots with an explicit source export binding. The module
    /// index also retains private declarations for internal type expansion,
    /// but ordinary value/module imports may select only this admitted set.
    pub(super) typescript_exported_declarations: AHashSet<DeclarationSlot>,
    /// Export aliases such as `export default Foo` and `export { Foo as Bar }`
    /// retain the exact source declaration selected by the adapter. Re-export
    /// chains without a local declaration remain unresolved until a later
    /// module hop can prove one target.
    pub(super) typescript_export_aliases: TypeScriptModuleIndex,
    /// Cross-file re-exports retain their normalized target module and export
    /// spelling. Resolution follows this bounded table rather than selecting
    /// a terminal name from an unrelated source file.
    pub(super) typescript_reexport_targets: TypeScriptReexportIndex,
    /// Project/module resolver decisions keyed by importer and raw module
    /// specifier. Values are normalized source-module keys and are retained
    /// only when the existing bounded project resolver admitted a target.
    pub(super) direct_bases: AHashMap<(String, String), DirectBaseSet>,
    pub(super) direct_subtypes: AHashMap<(String, String), DirectSubtypeSet>,
    pub(super) members_by_owner: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    pub(super) rust_impl_associated_types: AHashMap<(String, String, String), AssociatedTypeSet>,
    pub(super) rust_impl_associated_trait_names: AHashMap<(String, String), WildcardModuleSet>,
    pub(super) rust_impl_traits: AHashMap<(String, String), RustImplTraitSet>,
    pub(super) inventory_by_qualified: AHashMap<(String, String), Vec<String>>,
    pub(super) aliases: AHashMap<(String, String), Vec<String>>,
    pub(super) rust_source_wildcard_targets: AHashSet<String>,
    pub(super) wildcard_bindings_by_scope: AHashMap<(String, String), WildcardModuleSet>,
    pub(super) wildcard_bindings_by_module: AHashMap<(String, String), WildcardModuleSet>,
    pub(super) wildcard_reexports_by_module: AHashMap<(String, String), WildcardModuleSet>,
    pub(super) members: AHashMap<(String, String, String), Vec<String>>,
    pub(super) typescript_member_aliases: AHashMap<(String, String), Vec<TypeScriptMemberAlias>>,
    pub(super) return_candidates_by_callable: AHashMap<(String, String), Vec<String>>,
    pub(super) outer_return_candidates_by_callable: AHashMap<(String, String), Vec<String>>,
}
