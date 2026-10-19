//! Complete bounded traversal of the supplied static export obligations.

use super::*;
use std::collections::VecDeque;

const MAX_EXPORT_DEPTH: usize = 64;
type ExportKey<'a> = (&'a str, &'a str, &'a str);

#[derive(Default)]
struct ExportWalk<'a> {
    pending: VecDeque<(ExportKey<'a>, usize)>,
    discovered: BTreeSet<ExportKey<'a>>,
    nodes: BTreeMap<ExportKey<'a>, ExportNode<'a>>,
}

#[derive(Default)]
struct ExportNode<'a> {
    slots: BTreeSet<DeclarationSlot>,
    groups: Vec<ExportGroup<'a>>,
}

struct ExportGroup<'a> {
    targets: Vec<ExportKey<'a>>,
    required: bool,
}

impl<'a> ExportWalk<'a> {
    fn enqueue(
        &mut self,
        key: ExportKey<'a>,
        depth: usize,
        work: &mut TypeWork,
        limit: usize,
    ) -> TypedResult<()> {
        work.charge(1)?;
        // Success waits for the whole pending graph. In-progress state never
        // means completed absence, while shared DAG/cycle work is deduplicated.
        if self.discovered.contains(&key) {
            return Ok(());
        }
        if depth > MAX_EXPORT_DEPTH {
            return Err(TypeFailure::Limit {
                resource: "typescript_export_depth",
                limit: MAX_EXPORT_DEPTH,
            });
        }
        if self.discovered.len() >= limit {
            return Err(TypeFailure::Limit {
                resource: "typescript_export_states",
                limit,
            });
        }
        self.discovered.insert(key);
        self.pending.push_back((key, depth));
        Ok(())
    }

    fn binding(
        &mut self,
        db: &'a ResolutionDb<'_>,
        binding_id: &str,
        exported: &'a str,
        depth: usize,
        work: &mut TypeWork,
    ) -> TypedResult<Vec<ExportKey<'a>>> {
        work.charge(1)?;
        // A namespace reexport requires its own visibility-preserving value
        // transition. It cannot be treated as an ordinary nominal export.
        if exported == "*" {
            return Err(TypeFailure::Unknown);
        }
        let binding = db
            .facts
            .bindings
            .get(binding_id)
            .ok_or(TypeFailure::Unknown)?;
        work.charge(1)?;
        let targets = db
            .indexes
            .typescript
            .binding_sources
            .get(binding_id)
            .ok_or(TypeFailure::Unknown)?;
        db.check_typed_candidates(targets.sources.len())?;
        if targets.unavailable {
            return Err(TypeFailure::Unknown);
        }
        let mut keys = Vec::new();
        for module in &targets.sources {
            for language in typescript_language_family(&binding.language) {
                work.charge(1)?;
                if db
                    .indexes
                    .typescript
                    .source_files
                    .contains(&(language.to_string(), module.clone()))
                {
                    self.enqueue(
                        (language, module, exported),
                        depth,
                        work,
                        db.budget.candidates_per_lookup(),
                    )?;
                    keys.push((*language, module.as_str(), exported));
                }
            }
        }
        if keys.is_empty() {
            Err(TypeFailure::Unknown)
        } else {
            Ok(keys)
        }
    }
    fn finish(
        &mut self,
        roots: &[ExportKey<'a>],
        db: &ResolutionDb<'_>,
        work: &mut TypeWork,
    ) -> TypedResult<BTreeSet<DeclarationSlot>> {
        // Reachability alone is insufficient: a required alternative may have
        // no export even when a different alternative yields one. Compute each
        // state's closure, including cycles, before checking binding groups.
        // Every pass/probe/copy spends the SAME receiver meter.
        loop {
            let mut updates = Vec::new();
            for (key, node) in &self.nodes {
                work.charge(1)?;
                let mut added = BTreeSet::new();
                for group in &node.groups {
                    for target in &group.targets {
                        work.charge(1)?;
                        let child = self.nodes.get(target).ok_or(TypeFailure::Unknown)?;
                        for slot in &child.slots {
                            work.charge(1)?;
                            if !node.slots.contains(slot) {
                                db.check_typed_candidates(
                                    node.slots.len()
                                        + added.len()
                                        + usize::from(!added.contains(slot)),
                                )?;
                                added.insert(*slot);
                            }
                        }
                    }
                }
                if !added.is_empty() {
                    updates.push((*key, added));
                }
            }
            if updates.is_empty() {
                break;
            }
            for (key, added) in updates {
                self.nodes
                    .get_mut(&key)
                    .ok_or(TypeFailure::Unknown)?
                    .slots
                    .extend(added);
            }
        }
        for node in self.nodes.values() {
            work.charge(1)?;
            for group in &node.groups {
                let mut empty = false;
                let mut nonempty = false;
                for target in &group.targets {
                    work.charge(1)?;
                    let child = self.nodes.get(target).ok_or(TypeFailure::Unknown)?;
                    empty |= child.slots.is_empty();
                    nonempty |= !child.slots.is_empty();
                }
                // One binding's physical resolution alternatives cannot lose
                // an empty variant. A known empty independent star branch is
                // valid absence; an empty explicit/named target is not.
                if empty && (group.required || nonempty) {
                    return Err(TypeFailure::Unknown);
                }
            }
        }
        let mut slots = BTreeSet::new();
        for root in roots {
            work.charge(1)?;
            let node = self.nodes.get(root).ok_or(TypeFailure::Unknown)?;
            if node.slots.is_empty() {
                return Err(TypeFailure::Unknown);
            }
            for slot in &node.slots {
                work.charge(1)?;
                db.check_typed_candidates(slots.len() + usize::from(!slots.contains(slot)))?;
                slots.insert(*slot);
            }
        }
        Ok(slots)
    }
}

impl ResolutionDb<'_> {
    pub(super) fn typed_binding_export_slots(
        &self,
        binding_id: &str,
        exported: &str,
        _candidate: &RelationshipCandidate,
        work: &mut TypeWork,
    ) -> TypedResult<BTreeSet<DeclarationSlot>> {
        let mut walk = ExportWalk::default();
        let roots = walk.binding(self, binding_id, exported, 0, work)?;
        while let Some(((language, module, exported), depth)) = walk.pending.pop_front() {
            work.charge(1)?;
            if !self
                .indexes
                .typescript
                .complete_export_sources
                .contains(&(language.to_owned(), module.to_owned()))
            {
                return Err(TypeFailure::Unknown);
            }
            let key = (language.to_owned(), module.to_owned(), exported.to_owned());
            let mut explicit = false;
            let mut node = ExportNode::default();
            work.charge(1)?;
            if let Some(values) = self.indexes.typescript.direct_exports.get(&key) {
                self.check_typed_candidates(values.len())?;
                explicit = true;
                for slot in values {
                    work.charge(1)?;
                    // Preserve every public identity until uniqueness. A
                    // final-member or nominal-kind filter must not erase a
                    // competing owning module alternative.
                    node.slots.insert(*slot);
                    self.check_typed_candidates(node.slots.len())?;
                }
            }
            work.charge(1)?;
            let named = self.indexes.typescript.reexport_bindings.get(&key);
            if let Some(bindings) = named {
                explicit = true;
                self.check_typed_candidates(bindings.len())?;
                for id in bindings {
                    work.charge(1)?;
                    let binding = self.facts.bindings.get(id).ok_or(TypeFailure::Unknown)?;
                    let (_, target_export) = binding
                        .qualified_target
                        .rsplit_once("::")
                        .ok_or(TypeFailure::Unknown)?;
                    node.groups.push(ExportGroup {
                        targets: walk.binding(self, id, target_export, depth + 1, work)?,
                        required: true,
                    });
                }
            }
            // Missing named targets already returned Unknown. A star cannot
            // rescue an unavailable explicit export of the requested name.
            if !explicit && exported != "default" {
                work.charge(1)?;
                if let Some(bindings) = self.indexes.typescript.reexport_bindings.get(&(
                    language.to_owned(),
                    module.to_owned(),
                    "*".to_owned(),
                )) {
                    self.check_typed_candidates(bindings.len())?;
                    for id in bindings {
                        work.charge(1)?;
                        node.groups.push(ExportGroup {
                            targets: walk.binding(self, id, exported, depth + 1, work)?,
                            required: false,
                        });
                    }
                }
            }
            walk.nodes.insert((language, module, exported), node);
        }

        walk.finish(&roots, self, work)
    }
}
