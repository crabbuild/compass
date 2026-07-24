use std::collections::{BTreeSet, VecDeque};
use std::path::Path;

use crate::{
    Completeness, MAX_IMPACT_DEPTH, MAX_TRAVERSED_CALL_EDGES, SnapshotReader, SnapshotSide,
    TestEvidence, TestEvidenceProvider,
};

/// Conservative static test mapping over resolved reverse-call relationships.
///
/// Static relationships can recommend exact test entities, but cannot establish
/// that the tests ran or that call resolution is globally complete. The
/// provider therefore reports `Partial` and never creates a test-gap claim.
pub struct StaticTestEvidence<'a> {
    snapshots: &'a dyn SnapshotReader,
    side: SnapshotSide,
}

impl<'a> StaticTestEvidence<'a> {
    #[must_use]
    pub fn new(snapshots: &'a dyn SnapshotReader, side: SnapshotSide) -> Self {
        Self { snapshots, side }
    }
}

impl TestEvidenceProvider for StaticTestEvidence<'_> {
    fn tests_for(&self, symbol_id: &str) -> TestEvidence {
        let mut exact_tests = BTreeSet::new();
        let mut visited = BTreeSet::from([symbol_id.to_owned()]);
        let mut queue = VecDeque::from([(symbol_id.to_owned(), 0_u8)]);
        let mut traversed = 0_usize;
        while let Some((symbol, distance)) = queue.pop_front() {
            let callers = match self.snapshots.reverse_callers(self.side, &symbol) {
                Ok(callers) => callers,
                Err(_) => continue,
            };
            for caller in callers {
                traversed += 1;
                if traversed > MAX_TRAVERSED_CALL_EDGES {
                    break;
                }
                if !visited.insert(caller.clone()) {
                    continue;
                }
                let function = self.snapshots.function(self.side, &caller).ok().flatten();
                let mapped_test = function.as_ref().is_some_and(|function| {
                    function.is_test || is_test_path(&function.anchor.source_file, &function.name)
                });
                let graph_test = self
                    .snapshots
                    .node(self.side, &caller)
                    .ok()
                    .flatten()
                    .as_ref()
                    .is_some_and(is_test_entity);
                if mapped_test || graph_test {
                    exact_tests.insert(caller.clone());
                }
                if distance < MAX_IMPACT_DEPTH {
                    queue.push_back((caller, distance + 1));
                }
            }
            if traversed > MAX_TRAVERSED_CALL_EDGES {
                break;
            }
        }

        let exact_tests = exact_tests.into_iter().collect::<Vec<_>>();
        TestEvidence {
            completeness: Completeness::Partial,
            suggested_tests: exact_tests.clone(),
            exact_tests,
        }
    }
}

fn is_test_path(source_file: &str, function_name: &str) -> bool {
    let source_file = source_file.replace('\\', "/");
    let lower = source_file.to_ascii_lowercase();
    let path = Path::new(&lower);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let function_name = function_name
        .rsplit('.')
        .next()
        .unwrap_or(function_name)
        .to_ascii_lowercase();
    lower.starts_with("tests/")
        || lower.contains("/tests/")
        || lower.starts_with("test/")
        || lower.contains("/test/")
        || (file_name.starts_with("test_") && file_name.ends_with(".py"))
        || file_name.ends_with("_test.py")
        || file_name.contains(".test.")
        || file_name.contains(".spec.")
        || file_name.ends_with("_test.rs")
        || (file_name.ends_with(".rs") && function_name.starts_with("test_"))
}

fn is_test_entity(node: &compass_model::NodeRecord) -> bool {
    let source_file = node.string("source_file").replace('\\', "/");
    if source_file.is_empty() {
        return false;
    }
    let lower = source_file.to_ascii_lowercase();
    is_test_path(&lower, node.label().trim_end_matches("()"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use compass_analysis::FunctionSummary;
    use compass_ir::ModuleIr;
    use compass_model::NodeRecord;
    use serde_json::{Map, Value};

    use super::*;
    use crate::SemanticDiffError;

    struct Fixture {
        callers: BTreeMap<String, Vec<String>>,
        nodes: BTreeMap<String, NodeRecord>,
    }

    impl SnapshotReader for Fixture {
        fn node(
            &self,
            _side: SnapshotSide,
            node_id: &str,
        ) -> Result<Option<NodeRecord>, SemanticDiffError> {
            Ok(self.nodes.get(node_id).cloned())
        }

        fn module(
            &self,
            _side: SnapshotSide,
            _source_file: &str,
        ) -> Result<Option<ModuleIr>, SemanticDiffError> {
            Ok(None)
        }

        fn summary(
            &self,
            _side: SnapshotSide,
            _symbol_id: &str,
        ) -> Result<Option<FunctionSummary>, SemanticDiffError> {
            Ok(None)
        }

        fn reverse_callers(
            &self,
            _side: SnapshotSide,
            symbol_id: &str,
        ) -> Result<Vec<String>, SemanticDiffError> {
            Ok(self.callers.get(symbol_id).cloned().unwrap_or_default())
        }
    }

    #[test]
    fn static_mapping_recommends_resolved_test_callers_without_claiming_a_gap() {
        let fixture = Fixture {
            callers: BTreeMap::from([
                ("target".to_owned(), vec!["helper".to_owned()]),
                ("helper".to_owned(), vec!["test_target".to_owned()]),
            ]),
            nodes: BTreeMap::from([(
                "test_target".to_owned(),
                NodeRecord {
                    id: "test_target".to_owned(),
                    attributes: Map::from_iter([
                        (
                            "label".to_owned(),
                            Value::String("test_target()".to_owned()),
                        ),
                        (
                            "source_file".to_owned(),
                            Value::String("tests/target_test.rs".to_owned()),
                        ),
                    ]),
                },
            )]),
        };
        let evidence = StaticTestEvidence::new(&fixture, SnapshotSide::New).tests_for("target");
        assert_eq!(evidence.completeness, Completeness::Partial);
        assert_eq!(evidence.exact_tests, ["test_target"]);
    }
}
