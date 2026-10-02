use compass_query::{ConnectionLayer, HotspotReport, sanitize_label};

#[must_use]
pub fn render_hotspots_text(report: &HotspotReport) -> String {
    let mut lines = vec![format!(
        "Hotspots: {} scoped symbols; structural{}{} evidence; distinct directional contacts.",
        report.selected_symbols,
        if report.included_layers.inferred {
            "+inferred"
        } else {
            ""
        },
        if report.included_layers.documents {
            "+documents"
        } else {
            ""
        }
    )];
    for (title, rows) in [
        ("Most connected", &report.most_connected),
        ("Most depended on", &report.most_depended_on),
    ] {
        if title == "Most depended on" && !report.directed {
            lines.push("Dependents unavailable: graph is undirected.".into());
            continue;
        }
        lines.push(format!("{title}:"));
        for row in rows {
            let layers = row
                .layers
                .iter()
                .map(|(layer, count)| {
                    let name = match layer {
                        ConnectionLayer::Structural => "structural",
                        ConnectionLayer::Inferred => "inferred",
                        ConnectionLayer::Document => "document",
                    };
                    format!("{name}:{count}")
                })
                .collect::<Vec<_>>()
                .join(",");
            let layers = if report.included_layers.inferred || report.included_layers.documents {
                format!(" layers={layers}")
            } else {
                String::new()
            };
            lines.push(format!(
                "  {} {}:{} connections={} dependents={}{layers}",
                sanitize_label(&row.name),
                sanitize_label(row.source_file.as_deref().unwrap_or("?")),
                sanitize_label(&row.source_location),
                row.connections,
                row.dependents
            ));
        }
    }
    lines.push(format!("{} symbols omitted; {} relationship records excluded. Counts cover published graph evidence. Use --limit 100 or --scope PATH for more detail.", report.omitted_symbols, report.excluded_relationship_records));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_hotspots_retain_locations_and_do_not_claim_undirected_dependents()
    -> Result<(), Box<dyn std::error::Error>> {
        let document = serde_json::from_value(serde_json::json!({"directed":false,"nodes":[
            {"id":"a","label":"Service\nspoof","kind":"class","source_file":"a.rs","source_location":"L2"},
            {"id":"b","label":"Client","kind":"function","source_file":"b.rs","source_location":"L3"}],
            "links":[{"source":"a","target":"b","relation":"calls"}]}))?;
        let report = compass_query::hotspots(
            &document,
            &compass_query::OverviewScope::new(Vec::new())?,
            Default::default(),
            10,
        )?;
        let text = render_hotspots_text(&report);
        assert!(text.contains("a.rs:L2"));
        assert!(text.contains("Dependents unavailable: graph is undirected"));
        assert!(!text.contains("\nspoof"));
        assert!(text.len().div_ceil(4) < 200);
        Ok(())
    }
}
