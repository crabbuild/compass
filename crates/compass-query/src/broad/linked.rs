//! Navigation through witnessed calls, kept separate from owner-local matching.
use super::call_work::ProbeBudget;
use super::*;
use crate::record_budget::RecordBudget;
use compass_model::code_graph::EdgeRecord;

const MAX_READS: u64 = 800;
const MAX_DEPTH: usize = 4;
const MAX_EXPANSIONS_PER_ROOT: usize = 16;
const MAX_FANOUT: usize = 8;
const MAX_FRONTIER: usize = 32;
const MAX_DEFERRED: usize = 128;
const MAX_SINGLE_CHOICES: usize = 8;
const GROUP_PREFERENCE: f64 = 40_000.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedCallEndpoint {
    pub id: String,
    pub name: String,
    pub source: SourceAnchor,
}

/// One exact AST witness, not an inventory of all proofs or alternate paths.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedCallHop {
    pub edge_id: String,
    pub source_id: String,
    pub target: RankedCallEndpoint,
    pub site: SourceAnchor,
    pub occurrence_rule: Option<String>,
    pub extractor: String,
    pub rule: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedCallHint {
    pub requested: String,
    pub spelling: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RankedCallLink {
    pub schema: String,
    pub root: RankedCallEndpoint,
    pub hops: Vec<RankedCallHop>,
    pub hint: RankedCallHint,
}

fn scalar(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum
}

impl RankedCallLink {
    /// Validate the compact projection's bounds and chain coherence. The native
    /// producer separately verifies every identity and AST proof in its graph.
    pub fn is_coherent_for(&self, target: &str) -> bool {
        if self.schema != "compass.query.call-link/1"
            || self.hops.is_empty()
            || self.hops.len() > MAX_DEPTH
            || !endpoint_valid(&self.root)
            || !scalar(&self.hint.requested, 128)
            || !scalar(&self.hint.spelling, 128)
        {
            return false;
        }
        let mut owner = &self.root;
        let mut ids = BTreeSet::from([owner.id.as_str()]);
        let mut edge_ids = BTreeSet::new();
        for hop in &self.hops {
            if hop.source_id != owner.id
                || !scalar(&hop.edge_id, 512)
                || !edge_ids.insert(hop.edge_id.as_str())
                || hop.site.file.len() > 4096
                || !endpoint_valid(&hop.target)
                || !ids.insert(hop.target.id.as_str())
                || !relationships::encloses(&owner.source, &hop.site)
                || !scalar(&hop.extractor, 1024)
                || hop.rule.as_ref().is_some_and(|value| !scalar(value, 1024))
                || hop
                    .occurrence_rule
                    .as_ref()
                    .is_some_and(|value| !scalar(value, 1024))
            {
                return false;
            }
            owner = &hop.target;
        }
        owner.id == target
    }
}

fn endpoint_valid(endpoint: &RankedCallEndpoint) -> bool {
    scalar(&endpoint.id, 512)
        && scalar(&endpoint.name, 4096)
        && endpoint.source.file.len() <= 4096
        && endpoint.source.is_valid()
}

fn bounded_proofs(node: &NodeRecord) -> bool {
    node.evidence.len() <= 64
        && node.evidence.iter().all(|proof| {
            proof.anchors.len() <= 64
                && proof.candidates.len() <= 1
                && proof.extractor.len() <= 1024
                && proof.rule.as_ref().is_none_or(|rule| rule.len() <= 1024)
                && proof.anchors.iter().all(|source| source.file.len() <= 4096)
                && proof
                    .wiring_site
                    .as_ref()
                    .is_none_or(|source| source.file.len() <= 4096)
                && proof.candidates.iter().all(|candidate| {
                    candidate.node_id.len() <= 512
                        && candidate.reason.len() <= 1024
                        && candidate
                            .anchor
                            .as_ref()
                            .is_none_or(|source| source.file.len() <= 4096)
                })
        })
}

fn endpoint(node: &NodeRecord) -> Option<RankedCallEndpoint> {
    let source = node.source.as_ref()?;
    if !scalar(&node.id, 512)
        || !scalar(&node.qualified_name, 4096)
        || source.file.len() > 4096
        || !source.is_valid()
    {
        return None;
    }
    Some(RankedCallEndpoint {
        id: node.id.clone(),
        name: node.qualified_name.clone(),
        source: source.clone(),
    })
}

#[derive(Default)]
pub(super) struct Plan {
    operations: BTreeMap<String, BTreeSet<String>>,
    // All action-looking groups remain ineligible as independent topic credit,
    // even when surface-form navigation hints select fewer requested heads.
    predicates: BTreeSet<String>,
}

impl Plan {
    pub fn new(
        groups: &[(String, Vec<String>, f64, u64)],
        action_terms: &BTreeSet<String>,
        question: &str,
        allowed: bool,
    ) -> Self {
        if !allowed {
            return Self::default();
        }
        let mut operations = BTreeMap::new();
        for (term, variants, _, _) in groups {
            // These are name-navigation hints inside an exact call path, not
            // equivalence, owner matches, topic evidence or confidence boosts.
            let hints: &[&str] = match term.as_str() {
                "detect" => &["detect", "check", "validate", "validation"],
                "rebuilt" | "rebuild" => &["rebuild", "build"],
                "publish" => &["publish"],
                _ if action_terms.contains(term) => &[],
                _ => continue,
            };
            operations.insert(
                term.clone(),
                variants
                    .iter()
                    .cloned()
                    .chain(hints.iter().map(|word| (*word).to_owned()))
                    .collect(),
            );
        }
        let predicates = operations.keys().cloned().collect();
        // The owning request bounds bytes and groups before this call. These
        // are surface-form lexical navigation hints, not an English parse or a
        // rewrite of the complete query/ranking groups. Unknown forms retain
        // the previous action-looking set rather than inventing a unique role.
        let words = compass_model::identifier_tokens(question);
        let canonical = words
            .iter()
            .cloned()
            .map(compass_model::canonical_code_token)
            .collect::<Vec<_>>();
        let mut requested = words
            .iter()
            .zip(&canonical)
            .filter(|(raw, key)| {
                operations.contains_key(*key)
                    && ((raw.len() > 4 && raw.ends_with("ed"))
                        || matches!(raw.as_str(), "rebuilt" | "written" | "sent"))
            })
            .map(|(_, key)| key.clone())
            .collect::<BTreeSet<_>>();
        if requested.is_empty() {
            for (index, word) in words.iter().enumerate() {
                let head = canonical.get(index + 1);
                if matches!(
                    word.as_str(),
                    "to" | "we"
                        | "you"
                        | "i"
                        | "they"
                        | "can"
                        | "could"
                        | "should"
                        | "would"
                        | "will"
                        | "must"
                        | "do"
                        | "does"
                ) && let Some(head) = head.filter(|head| operations.contains_key(*head))
                {
                    requested.insert(head.clone());
                }
            }
            if let Some(head) = canonical
                .first()
                .filter(|head| operations.contains_key(*head))
            {
                requested.insert(head.clone());
            }
        }
        for (raw, keys) in words.windows(3).zip(canonical.windows(3)) {
            if matches!(raw[1].as_str(), "and" | "or")
                && operations.contains_key(&keys[0])
                && operations.contains_key(&keys[2])
            {
                requested.insert(keys[0].clone());
                requested.insert(keys[2].clone());
            }
        }
        if !requested.is_empty() {
            operations.retain(|term, _| requested.contains(term));
        }
        Self {
            operations,
            predicates,
        }
    }

    pub fn enabled(&self) -> bool {
        !self.operations.is_empty()
    }

    pub fn signals(
        &self,
        node: &NodeRecord,
        document: &SearchDocument,
        groups: &[(String, Vec<String>, f64, u64)],
        budget: &mut RecordBudget,
    ) -> Result<Signals, QueryError> {
        if !self.enabled() || !node.kind.is_callable() {
            return Ok(Signals::default());
        }
        budget.node_header(node)?;
        if !bounded_proofs(node) || !relationships::exact_ast(node) {
            return Ok(Signals::default());
        }
        let name = identifier_search_terms(&node.name)
            .into_iter()
            .map(compass_model::canonical_code_token)
            .collect::<BTreeSet<_>>();
        let own = document
            .fields
            .iter()
            .filter(|(field, _)| {
                matches!(
                    **field,
                    "name"
                        | "signature"
                        | "body_identifier"
                        | "body_comment"
                        | "owned_identifier"
                        | "declaring_type"
                )
            })
            .flat_map(|(_, terms)| terms.iter().cloned())
            .collect::<BTreeSet<_>>();
        let subjects = groups
            .iter()
            .filter(|(term, variants, _, _)| {
                !self.predicates.contains(term)
                    && !is_decision_predicate(term)
                    && !matches!(term.as_str(), "all" | "only" | "new" | "made")
                    && variants.iter().any(|variant| own.contains(variant))
            })
            .count();
        let mut hints = BTreeMap::new();
        for (requested, variants) in &self.operations {
            if let Some(spelling) = variants.iter().find(|variant| name.contains(*variant)) {
                let diversity = variants.intersection(&own).count();
                hints.insert(requested.clone(), (spelling.clone(), diversity));
            }
        }
        Ok(Signals { subjects, hints })
    }

    pub fn permits(&self, signals: &Signals) -> bool {
        self.enabled() && !signals.hints.is_empty()
    }
}

#[derive(Default)]
pub(super) struct Signals {
    subjects: usize,
    hints: BTreeMap<String, (String, usize)>,
}
impl Signals {
    pub(super) fn is_informative(&self) -> bool {
        self.subjects > 0 || !self.hints.is_empty()
    }
}

pub(super) struct Deferred {
    pub candidate: RankedCandidate,
}

pub(super) fn retain_deferred(pool: &mut Vec<Deferred>, bounded: &mut bool) {
    if pool.len() <= MAX_DEFERRED {
        return;
    }
    pool.sort_by(|a, b| {
        b.candidate
            .score
            .total_cmp(&a.candidate.score)
            .then_with(|| a.candidate.node.id.cmp(&b.candidate.node.id))
    });
    pool.truncate(MAX_DEFERRED);
    *bounded = true;
}

#[derive(Default)]
pub(super) struct Witness {
    pub nodes: BTreeMap<String, QueryNode>,
    pub edges: BTreeMap<String, compass_model::query_contract::DiscoveryEdge>,
    pub expanded: u64,
    pub work: Option<RankedCallWork>,
}

impl Witness {
    pub(super) fn required_nodes(candidates: &[RankedCandidate]) -> BTreeSet<&str> {
        candidates
            .iter()
            .filter_map(|candidate| candidate.call_link.as_ref())
            .flat_map(|link| {
                std::iter::once(link.root.id.as_str())
                    .chain(link.hops.iter().map(|hop| hop.target.id.as_str()))
            })
            .collect()
    }

    pub(super) fn retain_for(&mut self, candidates: &[RankedCandidate]) {
        let nodes = Self::required_nodes(candidates);
        let edges = candidates
            .iter()
            .filter_map(|candidate| candidate.call_link.as_ref())
            .flat_map(|link| link.hops.iter().map(|hop| hop.edge_id.as_str()))
            .collect::<BTreeSet<_>>();
        self.nodes.retain(|id, _| nodes.contains(id.as_str()));
        self.edges.retain(|id, _| edges.contains(id.as_str()));
        // Presentation pruning never erases work already performed.
    }
}

pub(super) struct Work<'a> {
    pub indexed_probes: u64,
    pub reads: &'a mut u64,
    pub bounded: &'a mut bool,
    pub relationship_budget: u64,
    pub max_depth: u32,
    pub budget: &'a mut RecordBudget,
}

struct Choice {
    link: RankedCallLink,
    diversity: usize,
}

fn choice_order(candidate: &Choice, old: &Choice) -> std::cmp::Ordering {
    old.diversity
        .cmp(&candidate.diversity)
        .then_with(|| candidate.link.hops.len().cmp(&old.link.hops.len()))
        .then_with(|| {
            candidate
                .link
                .hops
                .iter()
                .map(|hop| &hop.edge_id)
                .cmp(old.link.hops.iter().map(|hop| &hop.edge_id))
        })
}

fn better(candidate: &Choice, old: &Choice) -> bool {
    choice_order(candidate, old).is_lt()
}

// Single-operation alternatives are distinct known call targets, not a unique
// semantic interpretation. Retain a bounded best set, never the first target.
// Every endpoint/path has already passed the native scalar/vector bounds.
fn retain_single_choice(
    choices: &mut BTreeMap<String, Choice>,
    target: &str,
    candidate: Choice,
    bounded: &mut bool,
) {
    if let Some(old) = choices.get_mut(target) {
        if better(&candidate, old) {
            *old = candidate;
        }
        return;
    }
    if choices.len() >= MAX_SINGLE_CHOICES {
        *bounded = true;
        let Some((weakest_key, weakest)) =
            choices
                .iter()
                .max_by(|(left_key, left), (right_key, right)| {
                    choice_order(left, right).then_with(|| left_key.cmp(right_key))
                })
        else {
            return;
        };
        if !better(&candidate, weakest) {
            return;
        }
        let key = weakest_key.clone();
        choices.remove(&key);
    }
    choices.insert(target.to_owned(), candidate);
}

fn witness(owner: &NodeRecord, target: &NodeRecord, edge: &EdgeRecord) -> Option<RankedCallHop> {
    // Check vector/string bounds before proof validation or copying.
    if edge.evidence.len() > 64
        || !scalar(&edge.id, 512)
        || edge
            .occurrence_rule
            .as_ref()
            .is_some_and(|rule| !scalar(rule.as_str(), 1024))
        || edge.evidence.iter().any(|proof| {
            proof.anchors.len() > 64
                || proof.anchors.iter().any(|source| source.file.len() > 4096)
                || proof
                    .wiring_site
                    .as_ref()
                    .is_some_and(|source| source.file.len() > 4096)
                || proof.candidates.len() > 1
                || !scalar(&proof.extractor, 1024)
                || proof.rule.as_ref().is_some_and(|rule| !scalar(rule, 1024))
                || proof.candidates.iter().any(|candidate| {
                    candidate.node_id != target.id
                        || candidate.reason.len() > 1024
                        || candidate
                            .anchor
                            .as_ref()
                            .is_some_and(|source| source.file.len() > 4096)
                        || candidate.confidence != EvidenceConfidence::Exact
                })
        })
    {
        return None;
    }
    let site = relationships::verified_site(owner, edge)?;
    let proof = edge
        .evidence
        .iter()
        .filter(|proof| proof.origin == EvidenceOrigin::Ast && proof.anchors.contains(site))
        .min_by_key(|proof| (&proof.extractor, &proof.rule))?;
    Some(RankedCallHop {
        edge_id: edge.id.clone(),
        source_id: owner.id.clone(),
        target: endpoint(target)?,
        site: site.clone(),
        occurrence_rule: edge
            .occurrence_rule
            .as_ref()
            .map(|rule| rule.as_str().to_owned()),
        extractor: proof.extractor.clone(),
        rule: proof.rule.clone(),
    })
}

// Preserve the selected native call site locally; historical query_edge keeps
// its established first-anchor projection. Every selected site below comes
// from the pinned raw proof rather than a synthesized compact attribution.
fn selected_edge_projection(
    edge: &EdgeRecord,
    hop: &RankedCallHop,
    budget: &mut RecordBudget,
) -> Result<compass_model::query_contract::DiscoveryEdge, QueryError> {
    budget.edge(edge)?;
    if edge.evidence.len() > 64 || edge.evidence.iter().any(|proof| proof.anchors.len() > 64) {
        return Err(QueryError::new(
            QueryErrorKind::ExpansionLimit,
            "linked_witness_evidence_limit",
            "Selected call witnesses support at most 64 proofs with 64 anchors each",
        ));
    }
    let selected_at = edge
        .evidence
        .iter()
        .position(|proof| {
            proof.origin == EvidenceOrigin::Ast
                && proof.confidence == EvidenceConfidence::Exact
                && proof.extractor == hop.extractor
                && proof.rule == hop.rule
                && proof.anchors.contains(&hop.site)
        })
        .ok_or_else(|| {
            QueryError::new(
                QueryErrorKind::GraphInvariant,
                "linked_selected_proof_missing",
                "A pinned call occurrence lost its selected native AST attribution",
            )
        })?;
    let mut projected = crate::discovery::discovery_edge(edge);
    let selected = projected.evidence.get(selected_at).ok_or_else(|| {
        QueryError::new(
            QueryErrorKind::GraphInvariant,
            "linked_selected_projection_missing",
            "Native call projection lost its selected proof",
        )
    })?;
    budget.projection(selected)?;
    let mut selected = selected.clone();
    selected.anchor = Some(hop.site.clone());
    if !projected.evidence.contains(&selected) {
        projected.evidence.push(selected);
    }
    Ok(projected)
}

/// An operation group is navigation evidence. Only the originally weak
/// answer is reordered; strong/literal answers never enter this path.
pub(super) struct Context<'a, 'b> {
    pub backend: &'a PinnedDiscoveryBackend<'b>,
    pub plan: &'a Plan,
    pub signals: &'a BTreeMap<String, Signals>,
    pub scopes: &'a [DiscoveryScope],
    pub check: &'a dyn Fn() -> Result<(), QueryError>,
}

impl Context<'_, '_> {
    pub(super) fn apply(
        self,
        ranked: &mut Vec<RankedCandidate>,
        deferred: &mut Vec<Deferred>,
        work: Work<'_>,
    ) -> Result<Witness, QueryError> {
        let Self {
            backend,
            plan,
            signals,
            scopes,
            check,
        } = self;
        if !plan.enabled()
            || ranked.is_empty()
            || deferred.is_empty()
            || ranked
                .iter()
                .any(|candidate| candidate.relevance != RelevanceBand::Low)
        {
            return Ok(Witness::default());
        }
        let single_operation = plan.operations.len() == 1;
        let minimum_helpers = if single_operation { 1 } else { 2 };
        // Internal path enumeration is BFS even when later context presentation
        // requests DFS. Both are bounded by the caller's validated depth.
        let max_depth = (work.max_depth as usize).min(MAX_DEPTH);
        let reads_before = *work.reads;
        let mut probes = ProbeBudget::new(work.indexed_probes);
        let mut expanded = 0_u64;
        let targets = deferred
            .iter()
            .map(|item| item.candidate.node.id.as_str())
            .collect::<BTreeSet<_>>();
        let mut best: Option<(usize, usize, usize, BTreeMap<String, Choice>)> = None;
        for (root_rank, candidate) in ranked.iter().take(5).enumerate() {
            if probes.exhausted() {
                *work.bounded = true;
                break;
            }
            let Some(root_signal) = signals.get(&candidate.node.id) else {
                continue;
            };
            if root_signal.subjects < 2 {
                continue;
            }
            check()?;
            if work.reads.saturating_add(18) > FIELD_READ_CEILING
                || work.reads.saturating_add(18).saturating_sub(reads_before) > MAX_READS
            {
                *work.bounded = true;
                break;
            }
            *work.reads += 1;
            let Some(root_record) = backend.node_for_link(&candidate.node.id, work.budget)? else {
                continue;
            };
            if test_context(&root_record) {
                continue;
            }
            let Some(root) = endpoint(&root_record) else {
                *work.bounded = true;
                continue;
            };
            let mut queue = VecDeque::from([(root.id.clone(), Vec::<RankedCallHop>::new())]);
            let mut seen = BTreeSet::from([root.id.clone()]);
            let mut expansions = 0;
            let mut choices = BTreeMap::<String, Choice>::new();
            while let Some((id, prefix)) = queue.pop_front() {
                check()?;
                if prefix.len() >= max_depth
                    || expansions >= MAX_EXPANSIONS_PER_ROOT
                    || expanded >= work.relationship_budget
                    || work.reads.saturating_add(35) > FIELD_READ_CEILING
                    || work.reads.saturating_add(35).saturating_sub(reads_before) > MAX_READS
                {
                    *work.bounded = true;
                    break;
                }
                expansions += 1;
                *work.reads += 1;
                let Some(owner) = backend.node_for_link(&id, work.budget)? else {
                    continue;
                };
                if !owner.kind.is_callable()
                    || !bounded_proofs(&owner)
                    || !relationships::exact_ast(&owner)
                {
                    continue;
                }
                let fanout = work
                    .relationship_budget
                    .saturating_sub(expanded)
                    .saturating_sub(1)
                    .min(MAX_FANOUT as u64) as usize;
                let Some((edges, limited, examined)) =
                    probes.run(|| backend.calls_for_link(&id, fanout, work.budget))?
                else {
                    *work.bounded = true;
                    break;
                };
                expanded += examined as u64;
                *work.reads += examined as u64;
                *work.bounded |= limited;
                for edge in edges {
                    check()?;
                    *work.reads += 1;
                    let Some(target) = backend.node_for_link(&edge.target, work.budget)? else {
                        continue;
                    };
                    if !target.kind.is_callable()
                        || !bounded_proofs(&target)
                        || !relationships::exact_ast(&target)
                        || !discovery_scope_matches(&target, scopes)
                        || test_context(&target)
                    {
                        continue;
                    }
                    let Some(hop) = witness(&owner, &target, &edge) else {
                        *work.bounded = true;
                        continue;
                    };
                    if target.id == root.id || prefix.iter().any(|hop| hop.target.id == target.id) {
                        continue;
                    }
                    let mut path = prefix.clone();
                    path.push(hop);
                    if targets.contains(target.id.as_str())
                        && let Some(signal) = signals.get(&target.id)
                    {
                        for (requested, (spelling, diversity)) in &signal.hints {
                            let choice = Choice {
                                link: RankedCallLink {
                                    schema: "compass.query.call-link/1".to_owned(),
                                    root: root.clone(),
                                    hops: path.clone(),
                                    hint: RankedCallHint {
                                        requested: requested.clone(),
                                        spelling: spelling.clone(),
                                    },
                                },
                                diversity: *diversity,
                            };
                            if single_operation {
                                retain_single_choice(
                                    &mut choices,
                                    &target.id,
                                    choice,
                                    work.bounded,
                                );
                            } else if choices
                                .get(requested)
                                .is_none_or(|old| better(&choice, old))
                            {
                                choices.insert(requested.clone(), choice);
                            }
                        }
                    }
                    if path.len() < max_depth && seen.insert(target.id.clone()) {
                        if queue.len() >= MAX_FRONTIER {
                            *work.bounded = true;
                        } else {
                            queue.push_back((target.id.clone(), path));
                        }
                    }
                }
            }
            if choices.len() < minimum_helpers {
                continue;
            }
            // More same-operation callees must not by themselves promote a root.
            let diversity = if single_operation {
                choices
                    .values()
                    .map(|choice| choice.diversity)
                    .max()
                    .unwrap_or(0)
            } else {
                choices.values().map(|choice| choice.diversity).sum()
            };
            if best.as_ref().is_none_or(|(subjects, hints, rank, _)| {
                (
                    root_signal.subjects,
                    diversity,
                    std::cmp::Reverse(root_rank),
                ) > (*subjects, *hints, std::cmp::Reverse(*rank))
            }) {
                best = Some((root_signal.subjects, diversity, root_rank, choices));
            }
        }
        let Some((_, _, root_rank, choices)) = best else {
            return Ok(Witness {
                expanded,
                work: Some(probes.finish()),
                ..Witness::default()
            });
        };
        let mut root = ranked.remove(root_rank);
        let mut helpers = Vec::new();
        let mut used = BTreeSet::new();
        let mut choices = choices.into_values().collect::<Vec<_>>();
        if single_operation {
            choices.sort_by(choice_order);
        }
        for choice in choices {
            let Some(id) = choice.link.hops.last().map(|hop| hop.target.id.clone()) else {
                continue;
            };
            if helpers.len() == 2 || !used.insert(id.clone()) {
                continue;
            }
            let Some(at) = deferred
                .iter()
                .position(|item| item.candidate.node.id == id)
            else {
                continue;
            };
            let mut helper = deferred.remove(at).candidate;
            helper.relevance = RelevanceBand::Low;
            helper.reasons.insert(0, "Source-witnessed call-linked inspection lead. Caller topic, name hints and path reachability do not establish this helper's requested behavior".to_owned());
            helper.call_link = Some(choice.link);
            helpers.push(helper);
        }
        if helpers.len() < minimum_helpers {
            ranked.insert(root_rank, root);
            return Ok(Witness {
                expanded,
                work: Some(probes.finish()),
                ..Witness::default()
            });
        }
        let mut witness = Witness::default();
        let mut node_ids = BTreeSet::new();
        let mut edge_ids = BTreeSet::new();
        for helper in &helpers {
            if let Some(link) = &helper.call_link {
                node_ids.insert(link.root.id.clone());
                for hop in &link.hops {
                    node_ids.insert(hop.target.id.clone());
                    edge_ids.insert(hop.edge_id.clone());
                }
            }
        }
        for id in node_ids {
            check()?;
            *work.reads += 1;
            let node = backend.node_for_link(&id, work.budget)?.ok_or_else(|| {
                QueryError::new(
                    QueryErrorKind::GraphInvariant,
                    "linked_node_missing",
                    "A witnessed helper path lost its pinned node",
                )
            })?;
            work.budget.node(&node)?;
            witness.nodes.insert(id, query_node(&node));
        }
        let selected_hops = helpers
            .iter()
            .filter_map(|helper| helper.call_link.as_ref())
            .flat_map(|link| &link.hops)
            .map(|hop| (hop.edge_id.as_str(), hop))
            .collect::<BTreeMap<_, _>>();
        *work.reads += edge_ids.len() as u64;
        for edge in backend.edges_for_link(&edge_ids, work.budget)? {
            check()?;
            let hop = selected_hops.get(edge.id.as_str()).ok_or_else(|| {
                QueryError::new(
                    QueryErrorKind::GraphInvariant,
                    "linked_occurrence_missing",
                    "A pinned call occurrence has no selected path witness",
                )
            })?;
            witness.edges.insert(
                edge.id.clone(),
                selected_edge_projection(&edge, hop, work.budget)?,
            );
        }
        witness.expanded = expanded;
        witness.work = Some(probes.finish());
        let reason = if single_operation {
            "Requested operation-name leads are reachable through exact AST call paths. Alternatives are bounded inspection candidates, not a unique interpretation or proof of requested behavior"
        } else {
            "Two requested operation-name leads are reachable through exact AST call paths. This receives an inspection preference; path reachability and lexical hints do not establish the requested behavior"
        };
        root.reasons.insert(0, reason.to_owned());
        root.score += GROUP_PREFERENCE;
        for (position, helper) in helpers.iter_mut().enumerate() {
            helper.score = root.score - (position + 1) as f64;
        }
        ranked.insert(0, root);
        for (position, helper) in helpers.into_iter().enumerate() {
            ranked.insert(position + 1, helper);
        }
        // Every retained path is one witness, never an exhaustive call census.
        *work.bounded = true;
        Ok(witness)
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;
    use std::error::Error;

    fn groups(question: &str) -> Vec<(String, Vec<String>, f64, u64)> {
        super::super::terms(question)
            .into_iter()
            .map(|term| {
                let mut variants = vec![term.clone()];
                if term == "acquir" {
                    variants.push("acquire".to_owned());
                }
                if term == "rebuilt" {
                    variants.push("rebuild".to_owned());
                }
                (term, variants, 1.0, 1)
            })
            .collect()
    }

    #[test]
    fn surface_hints_keep_noun_modifiers_and_explicit_conjoined_operations_separate() {
        for (question, expected) in [
            ("Where are record caches checked?", vec!["check"]),
            ("Where do we check record caches?", vec!["check"]),
            ("Check record caches", vec!["check"]),
            (
                "Where are record caches checked and published?",
                vec!["check", "publish"],
            ),
            (
                "Where do we check and publish record caches?",
                vec!["check", "publish"],
            ),
            (
                "Where do we record and check caches?",
                vec!["check", "record"],
            ),
            (
                "Where are record caches detected and rebuild?",
                vec!["detect", "rebuild"],
            ),
            ("Where are record caches acquired?", vec!["acquir"]),
            // No recognized surface head: preserve the prior classified set.
            ("About record caches check", vec!["check", "record"]),
        ] {
            let groups = groups(question);
            let actions = groups
                .iter()
                .filter(|(term, _, _, _)| is_retrieval_action(term) || term == "acquir")
                .map(|(term, _, _, _)| term.clone())
                .collect();
            let plan = Plan::new(&groups, &actions, question, true);
            assert_eq!(
                plan.operations
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                expected,
                "{question}"
            );
            assert!(!Plan::new(&groups, &actions, question, false).enabled());
        }
        assert_eq!(
            crate::ranking::canonical_predicate_token("record"),
            Some("persist")
        );
    }

    #[test]
    fn filtering_a_navigation_head_never_creates_new_topic_credit() -> Result<(), Box<dyn Error>> {
        let question = "Where are record caches checked?";
        let groups = groups(question);
        let actions = BTreeSet::from(["check".to_owned(), "record".to_owned()]);
        let plan = Plan::new(&groups, &actions, question, true);
        assert!(!plan.operations.contains_key("record"));
        assert!(plan.predicates.contains("record"));
        let node =
            crate::test_support::node("root", NodeKind::Function, "opaque", "fixture.opaque");
        let document = SearchDocument {
            fields: BTreeMap::from([(
                "body_comment",
                BTreeSet::from(["record".to_owned(), "cach".to_owned()]),
            )]),
            truncated: false,
        };
        let mut budget = RecordBudget::new();
        assert_eq!(
            plan.signals(&node, &document, &groups, &mut budget)?
                .subjects,
            1
        );
        Ok(())
    }

    // Synthetic compact records exercise bounded selection, not source truth.
    fn choice(index: usize, diversity: usize) -> Choice {
        let source = SourceAnchor {
            file: "fixture.py".to_owned(),
            start_byte: 0,
            end_byte: 1,
            start_line: 1,
            end_line: 1,
            start_column: 0,
            end_column: 1,
        };
        let root = RankedCallEndpoint {
            id: "root".to_owned(),
            name: "root".to_owned(),
            source: source.clone(),
        };
        let target = RankedCallEndpoint {
            id: format!("helper-{index}"),
            name: format!("check_{index}"),
            source: source.clone(),
        };
        Choice {
            link: RankedCallLink {
                schema: "compass.query.call-link/1".to_owned(),
                root,
                hops: vec![RankedCallHop {
                    edge_id: format!("edge-{index}"),
                    source_id: "root".to_owned(),
                    target,
                    site: source,
                    occurrence_rule: None,
                    extractor: "authored-selector-fixture".to_owned(),
                    rule: None,
                }],
                hint: RankedCallHint {
                    requested: "check".to_owned(),
                    spelling: "check".to_owned(),
                },
            },
            diversity,
        }
    }

    #[test]
    fn single_alternatives_keep_eight_best_not_the_first_eight() {
        let mut choices = BTreeMap::new();
        let mut bounded = false;
        for index in 0..MAX_SINGLE_CHOICES {
            retain_single_choice(
                &mut choices,
                &format!("helper-{index}"),
                choice(index, index),
                &mut bounded,
            );
        }
        assert_eq!(choices.len(), MAX_SINGLE_CHOICES);
        assert!(!bounded);
        retain_single_choice(&mut choices, "helper-8", choice(8, 8), &mut bounded);
        assert!(bounded);
        assert_eq!(choices.len(), MAX_SINGLE_CHOICES);
        assert!(!choices.contains_key("helper-0"));
        let mut reversed = BTreeMap::new();
        let mut reversed_bound = false;
        for index in (0..=MAX_SINGLE_CHOICES).rev() {
            retain_single_choice(
                &mut reversed,
                &format!("helper-{index}"),
                choice(index, index),
                &mut reversed_bound,
            );
        }
        assert!(reversed_bound);
        assert_eq!(
            choices
                .values()
                .map(|choice| &choice.link)
                .collect::<Vec<_>>(),
            reversed
                .values()
                .map(|choice| &choice.link)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn same_target_paths_do_not_erase_distinct_alternatives() {
        let mut choices = BTreeMap::new();
        let mut bounded = false;
        retain_single_choice(&mut choices, "helper-1", choice(1, 1), &mut bounded);
        retain_single_choice(&mut choices, "helper-2", choice(2, 1), &mut bounded);
        retain_single_choice(&mut choices, "helper-1", choice(1, 3), &mut bounded);
        assert_eq!(choices.len(), 2);
        assert_eq!(
            choices.get("helper-1").map(|choice| choice.diversity),
            Some(3)
        );
        assert!(choices.contains_key("helper-2"));
        assert!(!bounded);
    }

    #[test]
    fn equal_diversity_prefers_shorter_paths_then_occurrence_ids() -> Result<(), Box<dyn Error>> {
        let copy_choice = |value: &Choice| Choice {
            link: value.link.clone(),
            diversity: value.diversity,
        };
        let mut short = choice(0, 1);
        let mut long = choice(0, 1);
        let hop = long.link.hops.first().ok_or("fixture hop missing")?.clone();
        long.link.hops.push(hop);
        short.link.hops[0].edge_id = "z-short".to_owned();
        long.link.hops[0].edge_id = "a-long".to_owned();
        assert!(better(&short, &long));
        let mut earlier = copy_choice(&short);
        earlier.link.hops[0].edge_id = "a-short".to_owned();
        assert!(better(&earlier, &short));
        for reverse in [false, true] {
            let mut choices = BTreeMap::new();
            let mut bounded = false;
            let mut inputs = vec![
                copy_choice(&long),
                copy_choice(&short),
                copy_choice(&earlier),
            ];
            if reverse {
                inputs.reverse();
            }
            for input in inputs {
                retain_single_choice(&mut choices, "helper-0", input, &mut bounded);
            }
            assert_eq!(
                choices.get("helper-0").map(|value| &value.link),
                Some(&earlier.link)
            );
            assert!(!bounded);
        }
        let mut choices = BTreeMap::new();
        let mut bounded = false;
        for index in 1..=MAX_SINGLE_CHOICES {
            retain_single_choice(
                &mut choices,
                &format!("helper-{index}"),
                choice(index, 1),
                &mut bounded,
            );
        }
        retain_single_choice(
            &mut choices,
            "helper-0",
            copy_choice(&earlier),
            &mut bounded,
        );
        assert!(bounded);
        assert_eq!(choices.len(), MAX_SINGLE_CHOICES);
        assert_eq!(
            choices.get("helper-0").map(|value| &value.link),
            Some(&earlier.link)
        );
        Ok(())
    }

    #[test]
    fn an_action_alias_cannot_erase_independent_noun_topic_credit() -> Result<(), Box<dyn Error>> {
        let groups = vec![
            (
                "database".to_owned(),
                vec!["database".to_owned(), "store".to_owned()],
                1.0,
                1_u64,
            ),
            ("error".to_owned(), vec!["error".to_owned()], 1.0, 1_u64),
            ("check".to_owned(), vec!["check".to_owned()], 1.0, 1_u64),
        ];
        let actions = BTreeSet::from(["check".to_owned()]);
        let plan = Plan::new(
            &groups,
            &actions,
            "Where are database errors checked?",
            true,
        );
        let node =
            crate::test_support::node("root", NodeKind::Function, "opaque", "fixture.opaque");
        let document = SearchDocument {
            fields: BTreeMap::from([(
                "body_comment",
                BTreeSet::from(["database".to_owned(), "error".to_owned()]),
            )]),
            truncated: false,
        };
        let mut budget = RecordBudget::new();
        assert_eq!(
            plan.signals(&node, &document, &groups, &mut budget)?
                .subjects,
            2
        );
        Ok(())
    }
}
