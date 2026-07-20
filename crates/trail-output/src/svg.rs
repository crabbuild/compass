use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::Path;

use trail_files::write_text_atomic;
use trail_graph::Communities;
use trail_model::GraphDocument;

use crate::OutputError;

const COMMUNITY_COLORS: [&str; 10] = [
    "#4E79A7", "#F28E2B", "#E15759", "#76B7B2", "#59A14F", "#EDC948", "#B07AA1", "#FF9DA7",
    "#9C755F", "#BAB0AC",
];

#[derive(Clone, Debug)]
pub struct SvgOptions<'a> {
    pub community_labels: Option<&'a BTreeMap<usize, String>>,
    pub width_inches: f64,
    pub height_inches: f64,
}

impl Default for SvgOptions<'_> {
    fn default() -> Self {
        Self {
            community_labels: None,
            width_inches: 20.0,
            height_inches: 14.0,
        }
    }
}

/// NetworkX-compatible force layout for the deterministic (`n < 500`) path.
/// Larger graphs retain the same force model, avoiding Python/SciPy at runtime.
#[must_use]
pub fn spring_layout(document: &GraphDocument) -> BTreeMap<String, (f64, f64)> {
    let count = document.nodes.len();
    if count == 0 {
        return BTreeMap::new();
    }
    if count == 1 {
        return BTreeMap::from([(document.nodes[0].id.clone(), (0.0, 0.0))]);
    }
    let indices = document
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut adjacency = vec![vec![0.0; count]; count];
    for edge in &document.links {
        let (Some(&source), Some(&target)) = (
            indices.get(edge.source.as_str()),
            indices.get(edge.target.as_str()),
        ) else {
            continue;
        };
        let weight = edge
            .attributes
            .get("weight")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(1.0);
        adjacency[source][target] = weight;
        if !document.directed {
            adjacency[target][source] = weight;
        }
    }
    let mut random = NumpyRandom::new(42);
    let mut positions = (0..count)
        .map(|_| [random.random(), random.random()])
        .collect::<Vec<_>>();
    let k = 2.0 / ((count as f64).sqrt() + 1.0);
    let x_extent = extent(&positions, 0);
    let y_extent = extent(&positions, 1);
    let mut temperature = x_extent.max(y_extent) * 0.1;
    let step = temperature / 51.0;
    for _ in 0..50 {
        let mut displacement = vec![[0.0, 0.0]; count];
        for left in 0..count {
            for right in 0..count {
                let dx = positions[left][0] - positions[right][0];
                let dy = positions[left][1] - positions[right][1];
                let distance = dx.hypot(dy).max(0.01);
                let force = k * k / (distance * distance) - adjacency[left][right] * distance / k;
                displacement[left][0] += dx * force;
                displacement[left][1] += dy * force;
            }
        }
        let mut movement_squared = 0.0;
        for (position, force) in positions.iter_mut().zip(displacement) {
            let length = force[0].hypot(force[1]).max(0.01);
            let dx = force[0] * temperature / length;
            let dy = force[1] * temperature / length;
            position[0] += dx;
            position[1] += dy;
            movement_squared += dx * dx + dy * dy;
        }
        temperature -= step;
        if movement_squared.sqrt() / (count as f64) < 1e-4 {
            break;
        }
    }
    let mean_x = positions.iter().map(|position| position[0]).sum::<f64>() / count as f64;
    let mean_y = positions.iter().map(|position| position[1]).sum::<f64>() / count as f64;
    for position in &mut positions {
        position[0] -= mean_x;
        position[1] -= mean_y;
    }
    let limit = positions
        .iter()
        .flat_map(|position| [position[0].abs(), position[1].abs()])
        .fold(0.0_f64, f64::max);
    if limit > 0.0 {
        for position in &mut positions {
            position[0] /= limit;
            position[1] /= limit;
        }
    }
    document
        .nodes
        .iter()
        .zip(positions)
        .map(|(node, position)| (node.id.clone(), (position[0], position[1])))
        .collect()
}

#[must_use]
pub fn svg_document(
    document: &GraphDocument,
    communities: &Communities,
    options: &SvgOptions<'_>,
) -> String {
    let width = (options.width_inches.max(1.0) * 72.0).round();
    let height = (options.height_inches.max(1.0) * 72.0).round();
    let margin = 55.0;
    let positions = spring_layout(document);
    let coordinates = positions
        .iter()
        .map(|(id, (x, y))| {
            (
                id.as_str(),
                (
                    margin + (x + 1.0) * 0.5 * (width - margin * 2.0),
                    margin + (1.0 - (y + 1.0) * 0.5) * (height - margin * 2.0),
                ),
            )
        })
        .collect::<HashMap<_, _>>();
    let node_community = communities
        .iter()
        .flat_map(|(community, members)| {
            members
                .iter()
                .map(move |member| (member.as_str(), *community))
        })
        .collect::<HashMap<_, _>>();
    let mut degree = document
        .nodes
        .iter()
        .map(|node| (node.id.as_str(), 0_usize))
        .collect::<HashMap<_, _>>();
    for edge in &document.links {
        *degree.entry(edge.source.as_str()).or_default() += 1;
        *degree.entry(edge.target.as_str()).or_default() += 1;
    }
    let max_degree = degree.values().copied().max().unwrap_or(1).max(1);
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}pt\" height=\"{height:.0}pt\" viewBox=\"0 0 {width:.0} {height:.0}\" role=\"img\" aria-label=\"Trail knowledge graph\">\n<rect width=\"100%\" height=\"100%\" fill=\"#1a1a2e\"/>\n<g id=\"edges\">\n"
    );
    for edge in &document.links {
        let (Some((x1, y1)), Some((x2, y2))) = (
            coordinates.get(edge.source.as_str()),
            coordinates.get(edge.target.as_str()),
        ) else {
            continue;
        };
        let extracted = edge
            .attributes
            .get("confidence")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("EXTRACTED")
            == "EXTRACTED";
        let dash = if extracted {
            ""
        } else {
            " stroke-dasharray=\"5 4\""
        };
        let opacity = if extracted { 0.6 } else { 0.3 };
        let _ = writeln!(
            svg,
            "<line x1=\"{x1:.3}\" y1=\"{y1:.3}\" x2=\"{x2:.3}\" y2=\"{y2:.3}\" stroke=\"#aaaaaa\" stroke-width=\"0.8\" opacity=\"{opacity}\"{dash}/>",
        );
    }
    svg.push_str("</g>\n<g id=\"nodes\">\n");
    for node in &document.nodes {
        let Some((x, y)) = coordinates.get(node.id.as_str()) else {
            continue;
        };
        let community = node_community.get(node.id.as_str()).copied().unwrap_or(0);
        let color = COMMUNITY_COLORS[community % COMMUNITY_COLORS.len()];
        let value = degree.get(node.id.as_str()).copied().unwrap_or(1);
        let area = 300.0 + 1200.0 * value as f64 / max_degree as f64;
        let radius = (area / std::f64::consts::PI).sqrt() * 0.55;
        let label = xml_escape(node.label());
        let _ = writeln!(
            svg,
            "<g class=\"node community-{community}\"><title>{label}</title><circle cx=\"{x:.3}\" cy=\"{y:.3}\" r=\"{radius:.3}\" fill=\"{color}\" fill-opacity=\"0.9\"/><text x=\"{x:.3}\" y=\"{:.3}\" text-anchor=\"middle\" fill=\"white\" font-family=\"sans-serif\" font-size=\"7\">{label}</text></g>",
            y + 2.5
        );
    }
    svg.push_str("</g>\n");
    if let Some(labels) = options.community_labels {
        svg.push_str("<g id=\"legend\" transform=\"translate(18 18)\">\n<rect x=\"-8\" y=\"-10\" width=\"260\" height=\"");
        let _ = write!(svg, "{}", labels.len() * 18 + 16);
        svg.push_str("\" fill=\"#2a2a4e\" fill-opacity=\"0.7\" rx=\"4\"/>\n");
        for (index, (community, label)) in labels.iter().enumerate() {
            let y = index * 18;
            let color = COMMUNITY_COLORS[community % COMMUNITY_COLORS.len()];
            let count = communities.get(community).map(Vec::len).unwrap_or_default();
            let label = xml_escape(label);
            let _ = writeln!(
                svg,
                "<rect x=\"0\" y=\"{y}\" width=\"12\" height=\"12\" fill=\"{color}\"/><text x=\"18\" y=\"{}\" fill=\"white\" font-family=\"sans-serif\" font-size=\"8\">{label} ({count})</text>",
                y + 10
            );
        }
        svg.push_str("</g>\n");
    }
    svg.push_str("</svg>");
    svg
}

pub fn write_svg(
    document: &GraphDocument,
    communities: &Communities,
    output_path: impl AsRef<Path>,
    options: &SvgOptions<'_>,
) -> Result<(), OutputError> {
    write_text_atomic(output_path, &svg_document(document, communities, options))?;
    Ok(())
}

fn extent(positions: &[[f64; 2]], axis: usize) -> f64 {
    let (minimum, maximum) = positions.iter().map(|position| position[axis]).fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(minimum, maximum), value| (minimum.min(value), maximum.max(value)),
    );
    maximum - minimum
}

fn xml_escape(value: &str) -> String {
    value
        .chars()
        .filter(|character| matches!(*character, '\t' | '\n' | '\r') || (*character as u32) >= 0x20)
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

struct NumpyRandom {
    state: [u32; 624],
    index: usize,
}

impl NumpyRandom {
    fn new(seed: u32) -> Self {
        let mut state = [0_u32; 624];
        state[0] = seed;
        for index in 1..624 {
            state[index] = 1_812_433_253_u32
                .wrapping_mul(state[index - 1] ^ (state[index - 1] >> 30))
                .wrapping_add(index as u32);
        }
        Self { state, index: 624 }
    }

    fn random(&mut self) -> f64 {
        let left = (self.next_u32() >> 5) as u64;
        let right = (self.next_u32() >> 6) as u64;
        (left * 67_108_864 + right) as f64 / 9_007_199_254_740_992.0
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            for index in 0..624 {
                let value = (self.state[index] & 0x8000_0000)
                    | (self.state[(index + 1) % 624] & 0x7fff_ffff);
                self.state[index] = self.state[(index + 397) % 624]
                    ^ (value >> 1)
                    ^ if value & 1 == 0 { 0 } else { 0x9908_b0df };
            }
            self.index = 0;
        }
        let mut value = self.state[self.index];
        self.index += 1;
        value ^= value >> 11;
        value ^= (value << 7) & 0x9d2c_5680;
        value ^= (value << 15) & 0xefc6_0000;
        value ^= value >> 18;
        value
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde_json::json;

    use super::*;

    #[test]
    fn svg_is_deterministic_and_xml_safe() -> Result<(), Box<dyn Error>> {
        let graph: GraphDocument = serde_json::from_value(json!({
            "nodes":[{"id":"a","label":"<script>x</script>"},{"id":"b","label":"B"}],
            "links":[{"source":"a","target":"b","confidence":"INFERRED"}]
        }))?;
        let first = svg_document(&graph, &Communities::new(), &SvgOptions::default());
        let second = svg_document(&graph, &Communities::new(), &SvgOptions::default());
        assert_eq!(first, second);
        assert!(first.contains("&lt;script&gt;x&lt;/script&gt;"));
        assert!(first.contains("stroke-dasharray"));
        assert!(!first.contains("<script>x</script>"));
        Ok(())
    }
}
