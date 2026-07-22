use std::collections::{HashMap, HashSet};

use compass_model::{EdgeRecord, NodeRecord};
use serde_json::Value;
use sha1::{Digest, Sha1};
use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;

const ENTROPY_THRESHOLD: f64 = 2.5;
const MERGE_THRESHOLD: f64 = 92.0;
const COMMUNITY_BOOST: f64 = 5.0;
const MINHASH_MAX: u64 = 0xffff_ffff;
const MINHASH_PRIME: u64 = (1_u64 << 61) - 1;
const LSH_BANDS: usize = 14;
const LSH_ROWS: usize = 9;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DedupError {
    #[error(
        "deduplicate_entities: nodes span multiple repos {0:?}. Cross-project dedup is disabled — run dedup per-repo before merging."
    )]
    MultipleRepositories(Vec<String>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DedupStats {
    pub removed: usize,
    pub exact_merges: usize,
    pub fuzzy_merges: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DedupResult {
    pub nodes: Vec<NodeRecord>,
    pub edges: Vec<EdgeRecord>,
    pub stats: DedupStats,
    pub diagnostics: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AmbiguousPair {
    pub left: NodeRecord,
    pub right: NodeRecord,
    pub score: f64,
}

pub trait EntityTiebreaker: Send {
    /// Return one merge decision per pair. Missing decisions are treated as `false`.
    fn decide(&mut self, pairs: &[AmbiguousPair]) -> Vec<bool>;
}

/// Deduplicate semantic entities using Graphify's deterministic compatibility rules.
///
/// Code symbols remain ID-addressed and are never label-merged. LLM tie-breaking is
/// deliberately outside this native deterministic stage.
pub fn deduplicate_entities(
    nodes: &[NodeRecord],
    edges: &[EdgeRecord],
    communities: &HashMap<String, i64>,
) -> Result<DedupResult, DedupError> {
    deduplicate_entities_with_tiebreaker(nodes, edges, communities, None)
}

pub fn deduplicate_entities_with_tiebreaker(
    nodes: &[NodeRecord],
    edges: &[EdgeRecord],
    communities: &HashMap<String, i64>,
    tiebreaker: Option<&mut dyn EntityTiebreaker>,
) -> Result<DedupResult, DedupError> {
    validate_repository_scope(nodes)?;
    if nodes.len() <= 1 {
        return Ok(DedupResult {
            nodes: nodes.to_vec(),
            edges: edges.to_vec(),
            stats: DedupStats::default(),
            diagnostics: Vec::new(),
        });
    }

    let (unique_nodes, diagnostics) = collapse_id_collisions(nodes);
    if unique_nodes.len() <= 1 {
        return Ok(DedupResult {
            nodes: unique_nodes,
            edges: edges.to_vec(),
            stats: DedupStats::default(),
            diagnostics,
        });
    }

    let mut union_find = UnionFind::default();
    let mut exact_merges = 0;
    let mut by_norm = HashMap::<String, Vec<&NodeRecord>>::new();
    for node in &unique_nodes {
        if is_code(node) {
            continue;
        }
        let key = normalize_label(node.label());
        if !key.is_empty() {
            by_norm.entry(key).or_default().push(node);
        }
    }
    for group in by_norm.values().filter(|group| group.len() > 1) {
        let mut by_file = HashMap::<String, Vec<&NodeRecord>>::new();
        for node in group {
            by_file.entry(source_file(node)).or_default().push(node);
        }
        for (source, file_group) in by_file {
            if source.is_empty() || file_group.len() <= 1 {
                continue;
            }
            let winner = pick_winner(&file_group);
            for node in &file_group {
                union_find.union(&winner.id, &node.id);
            }
            exact_merges += file_group.len() - 1;
        }
    }

    let mut candidates = Vec::<&NodeRecord>::new();
    let mut seen_norms = HashSet::new();
    for node in &unique_nodes {
        if is_code(node) {
            continue;
        }
        let key = normalize_label(node.label());
        if !key.is_empty() && seen_norms.insert(key) && entropy(node.label()) >= ENTROPY_THRESHOLD {
            candidates.push(node);
        }
    }
    let fuzzy_merges = fuzzy_merge(&candidates, communities, &mut union_find);
    if let Some(tiebreaker) = tiebreaker {
        let pairs = ambiguous_pairs(&candidates, communities, &mut union_find);
        let decisions = tiebreaker.decide(&pairs);
        for (pair, _merge) in pairs.iter().zip(decisions).filter(|(_, merge)| *merge) {
            let winner = pick_winner(&[&pair.left, &pair.right]);
            union_find.union(&winner.id, &pair.left.id);
            union_find.union(&winner.id, &pair.right.id);
        }
    }

    let mut remap = HashMap::<String, String>::new();
    for members in union_find.components().values() {
        if members.len() <= 1 {
            continue;
        }
        let group = unique_nodes
            .iter()
            .filter(|node| members.contains(&node.id))
            .collect::<Vec<_>>();
        let Some(winner) = pick_winner_optional(&group) else {
            continue;
        };
        for member in members {
            if member != &winner.id {
                remap.insert(member.clone(), winner.id.clone());
            }
        }
    }
    if remap.is_empty() {
        return Ok(DedupResult {
            nodes: unique_nodes,
            edges: edges.to_vec(),
            stats: DedupStats::default(),
            diagnostics,
        });
    }

    let deduped_nodes = unique_nodes
        .into_iter()
        .filter(|node| !remap.contains_key(&node.id))
        .collect();
    let deduped_edges = rewire_edges(edges, &remap);
    Ok(DedupResult {
        nodes: deduped_nodes,
        edges: deduped_edges,
        stats: DedupStats {
            removed: remap.len(),
            exact_merges,
            fuzzy_merges,
        },
        diagnostics,
    })
}

fn ambiguous_pairs(
    candidates: &[&NodeRecord],
    communities: &HashMap<String, i64>,
    union_find: &mut UnionFind,
) -> Vec<AmbiguousPair> {
    let mut output = Vec::new();
    for (index, node) in candidates.iter().enumerate() {
        let norm_left = normalize_label(node.label());
        for neighbor in &candidates[index + 1..] {
            if union_find.find(&node.id) == union_find.find(&neighbor.id) {
                continue;
            }
            let norm_right = normalize_label(neighbor.label());
            let cross_file = source_file(node) != source_file(neighbor);
            let max_length = norm_left.chars().count().max(norm_right.chars().count());
            let mut score = if cross_file && max_length >= 12 {
                strsim::jaro(&norm_left, &norm_right) * 100.0
            } else {
                strsim::jaro_winkler(&norm_left, &norm_right) * 100.0
            };
            if is_variant_pair(&norm_left, &norm_right)
                || short_label_blocked(&norm_left, &norm_right, score)
                || strict_prefix_pair(&norm_left, &norm_right)
                || numeric_tokens_differ(&norm_left, &norm_right)
                || crossfile_fileanchored_blocked(node, neighbor)
            {
                continue;
            }
            if communities.get(&node.id) == communities.get(&neighbor.id)
                && communities.contains_key(&node.id)
                && norm_left.chars().count().min(norm_right.chars().count()) >= 12
            {
                score += COMMUNITY_BOOST;
            }
            if (75.0..MERGE_THRESHOLD).contains(&score) {
                output.push(AmbiguousPair {
                    left: (*node).clone(),
                    right: (*neighbor).clone(),
                    score,
                });
            }
        }
    }
    output
}

fn validate_repository_scope(nodes: &[NodeRecord]) -> Result<(), DedupError> {
    let mut repositories = nodes
        .iter()
        .filter_map(|node| string_attribute(node, "repo"))
        .filter(|repo| !repo.is_empty())
        .collect::<Vec<_>>();
    repositories.sort();
    repositories.dedup();
    if repositories.len() > 1 {
        return Err(DedupError::MultipleRepositories(repositories));
    }
    Ok(())
}

fn collapse_id_collisions(nodes: &[NodeRecord]) -> (Vec<NodeRecord>, Vec<String>) {
    let mut output = Vec::<NodeRecord>::new();
    let mut positions = HashMap::<String, usize>::new();
    let mut dropped = HashMap::<String, Vec<NodeRecord>>::new();
    for node in nodes {
        if node.id.is_empty() {
            continue;
        }
        if let Some(&position) = positions.get(&node.id) {
            if collision_rank(node) < collision_rank(&output[position]) {
                let incumbent = std::mem::replace(&mut output[position], node.clone());
                dropped.entry(node.id.clone()).or_default().push(incumbent);
            } else {
                dropped
                    .entry(node.id.clone())
                    .or_default()
                    .push(node.clone());
            }
        } else {
            positions.insert(node.id.clone(), output.len());
            output.push(node.clone());
        }
    }
    let mut diagnostics = Vec::new();
    for node in &output {
        let Some(losers) = dropped.get(&node.id) else {
            continue;
        };
        diagnostics.extend(collision_diagnostics(node, losers));
    }
    (output, diagnostics)
}

fn collision_diagnostics(survivor: &NodeRecord, losers: &[NodeRecord]) -> Vec<String> {
    let keep_file = source_file(survivor);
    let keep_label = survivor.label();
    let mut messages = Vec::new();
    for loser in losers {
        let lose_file = source_file(loser);
        let lose_label = loser.label();
        if lose_file == keep_file {
            if normalize_label(lose_label) != normalize_label(keep_label) {
                messages.push(format!(
                    "note: node '{}' was extracted twice from '{}' under different labels — keeping '{}', dropping '{}'.",
                    survivor.id, keep_file, keep_label, lose_label
                ));
            }
        } else if !(defines_id(survivor) && !defines_id(loser)) {
            messages.push(format!(
                "WARNING: node '{}' is minted by two different files — keeping '{}' from '{}', dropping '{}' from '{}'.",
                survivor.id, keep_label, keep_file, lose_label, lose_file
            ));
        }
    }
    messages
}

fn fuzzy_merge(
    candidates: &[&NodeRecord],
    communities: &HashMap<String, i64>,
    union_find: &mut UnionFind,
) -> usize {
    if candidates.len() < 2 {
        return 0;
    }
    let mut norms = HashMap::<String, String>::new();
    let mut sketches = HashMap::<String, [u64; 128]>::new();
    let mut tables = (0..LSH_BANDS)
        .map(|_| HashMap::<Vec<u8>, Vec<String>>::new())
        .collect::<Vec<_>>();
    let mut by_id = HashMap::new();
    for node in candidates {
        let normalized = normalize_label(node.label());
        let sketch = minhash(&normalized);
        for (band, table) in tables.iter_mut().enumerate() {
            table
                .entry(band_key(&sketch, band))
                .or_default()
                .push(node.id.clone());
        }
        norms.insert(node.id.clone(), normalized);
        sketches.insert(node.id.clone(), sketch);
        by_id.insert(node.id.as_str(), *node);
    }

    let mut merge_count = 0;
    for node in candidates {
        let Some(sketch) = sketches.get(&node.id) else {
            continue;
        };
        let mut neighbors = HashSet::<String>::new();
        for (band, table) in tables.iter().enumerate() {
            if let Some(ids) = table.get(&band_key(sketch, band)) {
                neighbors.extend(ids.iter().cloned());
            }
        }
        let mut neighbors = neighbors.into_iter().collect::<Vec<_>>();
        neighbors.sort();
        for neighbor_id in neighbors {
            if neighbor_id == node.id || union_find.find(&node.id) == union_find.find(&neighbor_id)
            {
                continue;
            }
            let Some(neighbor) = by_id.get(neighbor_id.as_str()).copied() else {
                continue;
            };
            let Some(norm_label) = norms.get(&node.id) else {
                continue;
            };
            let Some(neighbor_norm) = norms.get(&neighbor_id) else {
                continue;
            };
            let cross_file = source_file(node) != source_file(neighbor);
            let max_length = norm_label
                .chars()
                .count()
                .max(neighbor_norm.chars().count());
            let mut score = if cross_file && max_length >= 12 {
                strsim::jaro(norm_label, neighbor_norm) * 100.0
            } else {
                strsim::jaro_winkler(norm_label, neighbor_norm) * 100.0
            };
            if is_variant_pair(norm_label, neighbor_norm)
                || short_label_blocked(norm_label, neighbor_norm, score)
                || strict_prefix_pair(norm_label, neighbor_norm)
                || numeric_tokens_differ(norm_label, neighbor_norm)
                || crossfile_fileanchored_blocked(node, neighbor)
            {
                continue;
            }
            if communities.get(&node.id) == communities.get(&neighbor_id)
                && communities.contains_key(&node.id)
                && norm_label
                    .chars()
                    .count()
                    .min(neighbor_norm.chars().count())
                    >= 12
            {
                score += COMMUNITY_BOOST;
            }
            if score < MERGE_THRESHOLD {
                continue;
            }
            if norm_label == neighbor_norm && cross_file {
                continue;
            }
            let winner = pick_winner(&[node, neighbor]);
            union_find.union(&winner.id, &node.id);
            union_find.union(&winner.id, &neighbor.id);
            merge_count += 1;
        }
    }
    merge_count
}

fn rewire_edges(edges: &[EdgeRecord], remap: &HashMap<String, String>) -> Vec<EdgeRecord> {
    let mut output = Vec::new();
    for edge in edges {
        let mut rewritten = edge.clone();
        if let Some(source) = remap.get(&rewritten.source) {
            rewritten.source.clone_from(source);
        }
        if let Some(target) = remap.get(&rewritten.target) {
            rewritten.target.clone_from(target);
        }
        rewritten.attributes.remove("from");
        rewritten.attributes.remove("to");
        if rewritten.source != rewritten.target {
            output.push(rewritten);
        }
    }
    output
}

fn normalize_label(label: &str) -> String {
    let normalized = label.nfkc().collect::<String>();
    let folded = normalized.case_fold().collect::<String>();
    let mut output = String::with_capacity(folded.len());
    let mut separator = false;
    for character in folded.chars() {
        if character.is_alphanumeric() {
            if separator && !output.is_empty() {
                output.push(' ');
            }
            output.push(character);
            separator = false;
        } else {
            separator = true;
        }
    }
    output.trim().to_owned()
}

fn entropy(label: &str) -> f64 {
    let normalized = normalize_label(label);
    if normalized.is_empty() {
        return 0.0;
    }
    let mut frequencies = HashMap::<char, usize>::new();
    let mut length = 0;
    for character in normalized.chars() {
        *frequencies.entry(character).or_default() += 1;
        length += 1;
    }
    frequencies.values().fold(0.0, |sum, count| {
        let probability = *count as f64 / length as f64;
        sum - probability * probability.log2()
    })
}

fn is_variant_pair(left: &str, right: &str) -> bool {
    if left == right || left.chars().count().max(right.chars().count()) >= 12 {
        return false;
    }
    let Some((left_stem, left_suffix)) = variant_parts(left) else {
        return false;
    };
    let Some((right_stem, right_suffix)) = variant_parts(right) else {
        return false;
    };
    left_stem == right_stem && left_suffix != right_suffix
}

fn variant_parts(value: &str) -> Option<(&str, &str)> {
    let bytes = value.as_bytes();
    let suffix_start = if let Some(digit) = bytes.iter().rposition(u8::is_ascii_digit) {
        let mut start = digit;
        while start > 0 && bytes[start - 1].is_ascii_digit() {
            start -= 1;
        }
        if !bytes[start..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
        {
            return None;
        }
        start
    } else {
        bytes.len().checked_sub(2)?
    };
    if suffix_start == 0 || !bytes[suffix_start - 1].is_ascii_lowercase() {
        return None;
    }
    Some((&value[..suffix_start], &value[suffix_start..]))
}

fn short_label_blocked(left: &str, right: &str, score: f64) -> bool {
    let left_len = left.chars().count();
    let right_len = right.chars().count();
    if left_len.max(right_len) >= 12 {
        return false;
    }
    !(score >= 97.0 && left_len == right_len && strsim::damerau_levenshtein(left, right) <= 1)
}

fn strict_prefix_pair(left: &str, right: &str) -> bool {
    let (short, long) = if left.chars().count() <= right.chars().count() {
        (left, right)
    } else {
        (right, left)
    };
    short != long && long.starts_with(short)
}

fn numeric_tokens_differ(left: &str, right: &str) -> bool {
    if left == right {
        return false;
    }
    numeric_tokens(left) != numeric_tokens(right)
}

fn numeric_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in value.chars() {
        if character.is_ascii_digit() {
            current.push(character);
        } else if !current.is_empty() {
            tokens.push(trim_number(&current));
            current.clear();
        }
    }
    if !current.is_empty() {
        tokens.push(trim_number(&current));
    }
    tokens.sort();
    tokens
}

fn trim_number(value: &str) -> String {
    let trimmed = value.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn crossfile_fileanchored_blocked(left: &NodeRecord, right: &NodeRecord) -> bool {
    let anchored = |node: &NodeRecord| {
        matches!(
            string_attribute(node, "file_type").as_deref(),
            Some("rationale" | "document")
        )
    };
    (anchored(left) || anchored(right)) && source_file(left) != source_file(right)
}

fn is_code(node: &NodeRecord) -> bool {
    string_attribute(node, "file_type").as_deref() == Some("code")
}

fn source_file(node: &NodeRecord) -> String {
    string_attribute(node, "source_file").unwrap_or_default()
}

fn string_attribute(node: &NodeRecord, key: &str) -> Option<String> {
    node.attributes
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
}

fn defines_id(node: &NodeRecord) -> bool {
    let source = source_file(node).replace('\\', "/");
    if node.id.is_empty() || source.is_empty() {
        return false;
    }
    let last_slash = source.rfind('/');
    let stem = source.rfind('.').map_or(source.as_str(), |dot| {
        if last_slash.is_none_or(|slash| dot > slash) {
            &source[..dot]
        } else {
            source.as_str()
        }
    });
    let segments = stem
        .split('/')
        .map(slug_segment)
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    (0..segments.len()).any(|index| {
        let prefix = segments[index..].join("_");
        node.id == prefix || node.id.starts_with(&format!("{prefix}_"))
    })
}

fn slug_segment(value: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for character in value.case_fold() {
        if character.is_ascii_alphanumeric() {
            if separator && !output.is_empty() {
                output.push('_');
            }
            output.push(character);
            separator = false;
        } else {
            separator = true;
        }
    }
    output.trim_matches('_').to_owned()
}

fn collision_rank(node: &NodeRecord) -> (bool, bool, usize, String, String) {
    (
        !node.attributes.contains_key("implementation_hash"),
        !defines_id(node),
        node.label().chars().count(),
        node.label().to_owned(),
        source_file(node),
    )
}

fn pick_winner<'a>(nodes: &[&'a NodeRecord]) -> &'a NodeRecord {
    pick_winner_optional(nodes).unwrap_or(nodes[0])
}

fn pick_winner_optional<'a>(nodes: &[&'a NodeRecord]) -> Option<&'a NodeRecord> {
    nodes
        .iter()
        .copied()
        .min_by_key(|node| (has_chunk_suffix(&node.id), node.id.chars().count()))
}

fn has_chunk_suffix(value: &str) -> bool {
    let Some((_, suffix)) = value.rsplit_once("_c") else {
        return false;
    };
    !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Default)]
struct UnionFind {
    parent: HashMap<String, String>,
}

impl UnionFind {
    fn find(&mut self, value: &str) -> String {
        self.parent
            .entry(value.to_owned())
            .or_insert_with(|| value.to_owned());
        let mut current = value.to_owned();
        loop {
            let parent = self
                .parent
                .get(&current)
                .cloned()
                .unwrap_or_else(|| current.clone());
            if parent == current {
                break;
            }
            let grandparent = self
                .parent
                .get(&parent)
                .cloned()
                .unwrap_or_else(|| parent.clone());
            self.parent.insert(current, grandparent.clone());
            current = grandparent;
        }
        current
    }

    fn union(&mut self, left: &str, right: &str) {
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root != right_root {
            self.parent.insert(right_root, left_root);
        }
    }

    fn components(&mut self) -> HashMap<String, Vec<String>> {
        let keys = self.parent.keys().cloned().collect::<Vec<_>>();
        let mut groups = HashMap::<String, Vec<String>>::new();
        for key in keys {
            let root = self.find(&key);
            groups.entry(root).or_default().push(key);
        }
        groups
    }
}

fn minhash(value: &str) -> [u64; 128] {
    let compact = value.replace(' ', "");
    let characters = compact.char_indices().collect::<Vec<_>>();
    let mut shingles = HashSet::<String>::new();
    if characters.len() < 3 {
        shingles.insert(compact);
    } else {
        for index in 0..=characters.len() - 3 {
            let start = characters[index].0;
            let end = characters
                .get(index + 3)
                .map_or(compact.len(), |(position, _)| *position);
            shingles.insert(compact[start..end].to_owned());
        }
    }
    let mut values = [MINHASH_MAX; 128];
    for shingle in shingles {
        let digest = Sha1::digest(shingle.as_bytes());
        let hash = u32::from_le_bytes([digest[0], digest[1], digest[2], digest[3]]) as u64;
        for index in 0..128 {
            let permuted = (MINHASH_A[index]
                .wrapping_mul(hash)
                .wrapping_add(MINHASH_B[index])
                % MINHASH_PRIME)
                & MINHASH_MAX;
            values[index] = values[index].min(permuted);
        }
    }
    values
}

fn band_key(sketch: &[u64; 128], band: usize) -> Vec<u8> {
    let start = band * LSH_ROWS;
    let mut key = Vec::with_capacity(LSH_ROWS * 8);
    for value in &sketch[start..start + LSH_ROWS] {
        key.extend_from_slice(&value.to_le_bytes());
    }
    key
}

#[rustfmt::skip]
const MINHASH_A: [u64; 128] = [775169054918279404,1758426461858698313,2109959069025162,965365488286768774,401325382989534145,1703346441743126658,1130051441076870728,1762784241922636285,401538927472639258,716042322565387541,815244983797985638,1110853719534651861,1465635305079316733,57506563652686493,505211975917066639,838727479550634565,780385900265431000,1082636226378482418,283838892089191903,1348484356917986391,936072031995811818,1720372350978982920,1169969703377029950,1241883087124931898,25625226007148417,361679113657972946,1568788725991793369,720433610630797940,826975332961259499,57968974610545097,1814178924063827016,850535481772967367,1529041984443575858,611791605100360196,1233664984050117401,1208550132088437804,1134491820483890203,1561068142528832849,337359223002198900,2662096424693203,2100366343476833345,2272005282452877338,561592535218511587,724575788541486984,1904183826989973146,1344648193880622103,619727673435799236,804538942828528767,696832086074912126,92760730228048028,357257325660623162,993090257743735975,1598006598579725601,286785215429783307,2150395256084530832,984365201973288024,1367228525499322312,400821414621316445,1647396391822001343,1379048364047586209,1887737050569905304,720455784159192386,1280206955645314894,722729898864855937,921478673934505214,662171862886500103,715634959063503318,274655559890433539,1283796700206534620,1596658653041296076,523922820580841550,230117649494758476,263352056799233068,1058682450511224351,418313753765229416,744403489731186478,968612313319578170,1803461786564664829,14980651099439201,1863084026736552971,153220345614122877,2281349539356141448,17384899557965977,1824352345109249501,367604757747313057,385282673040866498,979359603628154429,1657256394417291417,709120483056161652,2244889800033437827,2116689543321462493,1374084313144276835,1381884949962268343,1446033029236275690,611266146699233751,2066532958085233347,523509680078012392,1359146662721091127,52948149690579846,2160942293751064747,1413848896402184823,499133954986498327,201846032685059982,1978777819965811315,618614248984827574,2275592508583382027,291850381902561464,1004189732771057209,1215583606649627030,2256458337378950241,873278286232559572,223865076026037746,1062448801025099918,1324811268927594101,1217488311597731463,100775521383383737,71503144720579472,885896412999398396,1596100048030278083,2292394638421159734,366723709801908645,483507374032440057,522162562461265193,2235949585210378460,2029657444881885554,716306739587483086,974393427803837903,1697670337010853720];

#[rustfmt::skip]
const MINHASH_B: [u64; 128] = [2290593402415132572,538343888607264301,1581979269522776948,1744983512659606478,1125808846525888896,343981791039746111,934007963372074531,1991773790767872477,1053404851543353986,237237038733719123,2094477501324246838,2252746632473157885,1037449266289342519,211311645193258271,1105286383762747614,2238391591237301384,821837035338323555,1982916584933322248,1857759398970506189,1623110274942190159,1101583836342502208,231616986851349715,1327691181208577137,1702197838485543700,1256231807310832911,1444255381494669787,42902454742971891,2191950179549041915,1299194599953098987,1500807354596368775,1752809426824068149,1786176085821440878,2118567300926164723,2190914504203751524,2212792323624167200,1456940403417483715,586447722759106259,342670382210209374,1291680585167188655,2053954687016252293,1961432329506782783,1270084851437757527,1242617054144874282,2095848151477744322,1471039734164907746,94837956732254865,939828747546450752,2219778448427847156,815952863767273617,275217274352977059,1223661685133629084,2215162532383274790,87022673460369866,1571183205760722246,50831080378748134,1227381464362988592,190280048193181630,1010043456706110057,1262389667950943488,266756186393216503,455086835550584083,918794838454833776,245282199234122803,1216767326165072010,2030358702377275716,409112435725675790,605345743814266389,1050949148475592060,257361052164127160,2017374420125007056,2154180424385901110,1365360136487033415,1386754629279803918,1045066892334153871,749055048214673205,305592579266007545,276843534351670229,2047416485587801838,1924188903974864428,913997788746244246,944130688119949750,806739497089626636,366640366875901243,1628156380473706444,903072637067301591,24525324454322813,185927603917723084,104448596761453350,397062628907991664,1346062807897531205,1422529324905212321,277051278387475866,944404313172274117,1513805725792048436,110622250926641691,1097409548522419201,1747582826281098367,1201021145525391328,1512894580233772884,70687732435925863,1747542065935107222,560639205034286900,1565764496068337734,854947859192503591,896577955981603923,658896564124910768,2126052233424800198,1828262494289317028,286538330633190806,1724427592165531258,1082387680134353145,1058114924210953567,1203554243709668233,329988358101689785,1436212155308104045,1088546156183973597,699670876789814988,2066610522124628330,994450590275747945,833676259291159664,997394006025442462,804310844303211114,7416020676747193,1877124887546555817,29151700773516034,1822234730034014301,1931671111240692333,1454448473341514576];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, json};

    fn node(id: &str, label: &str, source_file: &str) -> NodeRecord {
        let mut attributes = Map::new();
        attributes.insert("label".to_owned(), json!(label));
        attributes.insert("source_file".to_owned(), json!(source_file));
        NodeRecord {
            id: id.to_owned(),
            attributes,
        }
    }

    #[test]
    fn exact_and_fuzzy_merges_rewire_edges() -> Result<(), DedupError> {
        let nodes = vec![
            node("user_service", "User Service", "a.md"),
            node("user_service_c1", "user_service", "a.md"),
            node("graph_extractor", "GraphExtractor", "a.md"),
            node("graphextractor", "Graph Extractor", "b.md"),
            node("parser", "Parser", "a.md"),
        ];
        let edges = vec![EdgeRecord {
            source: "graphextractor".to_owned(),
            target: "parser".to_owned(),
            attributes: Map::new(),
        }];
        let result = deduplicate_entities(&nodes, &edges, &HashMap::new())?;
        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.stats.removed, 2);
        assert_eq!(result.stats.exact_merges, 1);
        assert_eq!(result.stats.fuzzy_merges, 1);
        assert_eq!(result.edges[0].source, "graphextractor");
        Ok(())
    }

    #[test]
    fn variant_and_file_anchored_guards_preserve_entities() -> Result<(), DedupError> {
        let mut rationale_a = node(
            "r1",
            "Django app config for cards. No business logic here.",
            "a.py",
        );
        rationale_a
            .attributes
            .insert("file_type".to_owned(), json!("rationale"));
        let mut rationale_b = node(
            "r2",
            "Django app config for cores. No business logic here.",
            "b.py",
        );
        rationale_b
            .attributes
            .insert("file_type".to_owned(), json!("rationale"));
        let nodes = vec![
            node("m1", "ASR1603", "models.md"),
            node("m2", "ASR1605", "models.md"),
            rationale_a,
            rationale_b,
        ];
        let result = deduplicate_entities(&nodes, &[], &HashMap::new())?;
        assert_eq!(result.nodes.len(), 4);
        Ok(())
    }

    #[test]
    fn exact_id_collision_prefers_a_hashed_definition_over_a_declaration() -> Result<(), DedupError>
    {
        let mut definition = node(
            "db_db_impl_dbimpl_compact",
            "DBImpl::Compact()",
            "db/db_impl.cc",
        );
        definition.attributes.insert(
            "implementation_hash".to_owned(),
            json!("implementation-digest"),
        );
        let declaration = node("db_db_impl_dbimpl_compact", "Compact", "db/db_impl.h");

        let result = deduplicate_entities(&[definition, declaration], &[], &HashMap::new())?;

        assert_eq!(result.nodes.len(), 1);
        assert_eq!(source_file(&result.nodes[0]), "db/db_impl.cc");
        assert_eq!(
            result.nodes[0]
                .attributes
                .get("implementation_hash")
                .and_then(Value::as_str),
            Some("implementation-digest")
        );
        Ok(())
    }

    struct MergeAll {
        pairs_seen: usize,
    }

    impl EntityTiebreaker for MergeAll {
        fn decide(&mut self, pairs: &[AmbiguousPair]) -> Vec<bool> {
            self.pairs_seen = pairs.len();
            vec![true; pairs.len()]
        }
    }

    #[test]
    fn optional_tiebreaker_merges_only_ambiguous_pairs() -> Result<(), DedupError> {
        let nodes = vec![
            node("account", "Customer Account Management", "accounts.md"),
            node("identity", "Customer Identity Management", "identity.md"),
        ];
        let edges = vec![EdgeRecord {
            source: "identity".to_owned(),
            target: "account".to_owned(),
            attributes: Map::new(),
        }];
        let mut tiebreaker = MergeAll { pairs_seen: 0 };
        let result = deduplicate_entities_with_tiebreaker(
            &nodes,
            &edges,
            &HashMap::new(),
            Some(&mut tiebreaker),
        )?;
        assert_eq!(tiebreaker.pairs_seen, 1);
        assert_eq!(result.nodes.len(), 1);
        assert!(result.edges.is_empty());
        Ok(())
    }
}
