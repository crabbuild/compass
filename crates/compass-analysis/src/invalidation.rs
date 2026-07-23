use std::collections::{BTreeMap, BTreeSet, VecDeque};

use compass_ir::{ProgramBundle, SymbolId};

use crate::{AnalysisBundle, AnalysisError, summary::semantic_digest};

pub fn affected_summaries(
    previous: &AnalysisBundle,
    current: &ProgramBundle,
) -> Result<BTreeSet<SymbolId>, AnalysisError> {
    previous.validate()?;
    current.validate()?;

    let previous_digests = previous
        .summaries
        .iter()
        .map(|summary| (summary.symbol_id.clone(), summary.semantic_digest.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut current_digests = BTreeMap::new();
    let mut current_reverse = BTreeMap::<String, Vec<String>>::new();
    for module in &current.modules {
        for function in &module.functions {
            current_digests.insert(function.symbol_id.clone(), semantic_digest(function)?);
            for block in &function.blocks {
                for operation in &block.operations {
                    if let compass_ir::OperationKind::Call {
                        resolved_symbols, ..
                    } = &operation.kind
                        && resolved_symbols.len() == 1
                    {
                        current_reverse
                            .entry(resolved_symbols[0].clone())
                            .or_default()
                            .push(function.symbol_id.clone());
                    }
                }
            }
        }
    }

    let all_symbols = previous_digests
        .keys()
        .chain(current_digests.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut affected = all_symbols
        .into_iter()
        .filter(|symbol| previous_digests.get(symbol) != current_digests.get(symbol))
        .collect::<BTreeSet<_>>();
    let mut queue = affected.iter().cloned().collect::<VecDeque<_>>();
    while let Some(symbol) = queue.pop_front() {
        for caller in previous
            .reverse_calls
            .get(&symbol)
            .into_iter()
            .flatten()
            .chain(current_reverse.get(&symbol).into_iter().flatten())
        {
            if affected.insert(caller.clone()) {
                queue.push_back(caller.clone());
            }
        }
    }
    Ok(affected)
}
