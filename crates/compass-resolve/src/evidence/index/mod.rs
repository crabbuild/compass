//! Immutable, ownership-grouped secondary indexes over validated evidence.

use super::*;

mod builder;

#[derive(Default)]
pub(super) struct NameIndexes {
    pub(super) by_qualified: AHashMap<(String, String), Vec<DeclarationSlot>>,
    pub(super) by_module_name: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    pub(super) by_scope_name: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    pub(super) by_source_directory_name: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    pub(super) inventory_by_qualified: AHashMap<(String, String), Vec<String>>,
    pub(super) aliases: AHashMap<(String, String), Vec<String>>,
}

#[derive(Default)]
pub(super) struct HierarchyIndexes {
    pub(super) direct_bases: AHashMap<(String, String), DirectBaseSet>,
    pub(super) direct_subtypes: AHashMap<(String, String), DirectSubtypeSet>,
}

#[derive(Default)]
pub(super) struct MemberIndexes {
    pub(super) members_by_owner: AHashMap<(String, String, String), Vec<DeclarationSlot>>,
    pub(super) members: AHashMap<(String, String, String), Vec<String>>,
    pub(super) return_candidates_by_callable: AHashMap<(String, String), Vec<String>>,
    pub(super) outer_return_candidates_by_callable: AHashMap<(String, String), Vec<String>>,
}

#[derive(Default)]
pub(super) struct WildcardIndexes {
    pub(super) by_scope: AHashMap<(String, String), WildcardModuleSet>,
    pub(super) by_module: AHashMap<(String, String), WildcardModuleSet>,
    pub(super) reexports_by_module: AHashMap<(String, String), WildcardModuleSet>,
}

#[derive(Default)]
pub(super) struct TypeScriptIndexes {
    /// Repository-relative source module paths, never terminal basenames.
    pub(super) modules: TypeScriptModuleIndex,
    pub(super) exported_declarations: AHashSet<DeclarationSlot>,
    pub(super) export_aliases: TypeScriptModuleIndex,
    pub(super) reexport_targets: TypeScriptReexportIndex,
    pub(super) member_aliases: AHashMap<(String, String), Vec<TypeScriptMemberAlias>>,
}

#[derive(Default)]
pub(super) struct RustIndexes {
    pub(super) impl_associated_types: AHashMap<(String, String, String), AssociatedTypeSet>,
    pub(super) impl_associated_trait_names: AHashMap<(String, String), WildcardModuleSet>,
    pub(super) impl_traits: AHashMap<(String, String), RustImplTraitSet>,
    pub(super) source_wildcard_targets: AHashSet<String>,
}

#[derive(Clone)]
pub(super) struct CSharpBinding {
    pub(super) kind: compass_languages::BindingKind,
    pub(super) spelling: String,
    pub(super) qualified_target: String,
}

#[derive(Default)]
pub(super) struct CSharpIndexes {
    pub(super) bindings_by_source: AHashMap<String, Vec<CSharpBinding>>,
}

#[derive(Default)]
pub(super) struct PhpIndexes {
    pub(super) members_by_owner_folded: AHashMap<(String, String), Vec<DeclarationSlot>>,
}

#[derive(Default)]
pub(super) struct ResolutionIndexes {
    pub(super) names: NameIndexes,
    pub(super) hierarchy: HierarchyIndexes,
    pub(super) members: MemberIndexes,
    pub(super) wildcards: WildcardIndexes,
    pub(super) typescript: TypeScriptIndexes,
    pub(super) rust: RustIndexes,
    pub(super) csharp: CSharpIndexes,
    pub(super) php: PhpIndexes,
}
