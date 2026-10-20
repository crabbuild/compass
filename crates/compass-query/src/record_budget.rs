//! Borrowed metadata inspection before call-linked record cloning or validation.
//! Walking typed Serialize data checks string lengths without scanning contents;
//! collection lengths and nesting are checked before entering their children.
use serde::Serialize;
use serde::ser::{
    self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};

use crate::{QueryError, QueryErrorKind};
use compass_model::code_graph::{EdgeRecord, NodeRecord};
use compass_model::provenance::Provenance;

const MAX_EVIDENCE: usize = 8192;
pub(crate) const MAX_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_VALUES: usize = 262_144;
const MAX_DEPTH: usize = 32;

#[derive(Debug, thiserror::Error)]
enum InputError {
    #[error("call-linked metadata exceeds {resource} ({limit})")]
    Limit {
        resource: &'static str,
        limit: usize,
    },
    #[error("invalid call-linked metadata serialization")]
    Invalid,
}
impl ser::Error for InputError {
    fn custom<T: std::fmt::Display>(_message: T) -> Self {
        Self::Invalid
    }
}

type Result<T> = std::result::Result<T, InputError>;

pub(crate) struct RecordBudget {
    text: usize,
    values: usize,
    depth: usize,
    evidence: usize,
}
impl RecordBudget {
    pub(crate) fn new() -> Self {
        Self {
            text: MAX_TEXT_BYTES,
            values: MAX_VALUES,
            depth: MAX_DEPTH,
            evidence: MAX_EVIDENCE,
        }
    }
    fn text(&mut self, len: usize) -> Result<()> {
        self.text = self.text.checked_sub(len).ok_or(InputError::Limit {
            resource: "metadata text bytes",
            limit: MAX_TEXT_BYTES,
        })?;
        Ok(())
    }
    fn inspect<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        value.serialize(Inspector {
            budget: self,
            depth: 0,
        })
    }
}

fn limit(resource: &'static str, maximum: usize) -> QueryError {
    QueryError::new(
        QueryErrorKind::ExpansionLimit,
        "linked_record_metadata_limit",
        format!(
            "Call-linked inspection exceeds {resource} ({maximum}); narrow the question or source scope"
        ),
    )
}
fn mapped(error: InputError) -> QueryError {
    match error {
        InputError::Limit {
            resource,
            limit: maximum,
        } => limit(resource, maximum),
        InputError::Invalid => QueryError::new(
            QueryErrorKind::GraphInvariant,
            "linked_record_metadata_invalid",
            "Call-linked record metadata cannot be inspected",
        ),
    }
}
fn bounded(
    value: &str,
    maximum: usize,
    resource: &'static str,
) -> std::result::Result<(), QueryError> {
    if value.len() > maximum {
        Err(limit(resource, maximum))
    } else {
        Ok(())
    }
}
fn source(source: &compass_model::provenance::SourceAnchor) -> std::result::Result<(), QueryError> {
    bounded(&source.file, 4096, "source path bytes")
}
impl RecordBudget {
    pub(crate) fn identity(id: &str) -> std::result::Result<(), QueryError> {
        bounded(id, 512, "record identity bytes")
    }
    fn proofs(&mut self, proofs: &[Provenance]) -> std::result::Result<(), QueryError> {
        if proofs.len() > 64 {
            return Err(limit("proofs per record", 64));
        }
        self.evidence = self
            .evidence
            .checked_sub(proofs.len())
            .ok_or_else(|| limit("aggregate proof items", MAX_EVIDENCE))?;
        for proof in proofs {
            if proof.anchors.len() > 64 {
                return Err(limit("anchors per proof", 64));
            }
            if proof.candidates.len() > compass_model::provenance::MAX_RESOLUTION_CANDIDATES {
                return Err(limit(
                    "candidates per proof",
                    compass_model::provenance::MAX_RESOLUTION_CANDIDATES,
                ));
            }
            bounded(&proof.extractor, 1024, "extractor bytes")?;
            if let Some(rule) = &proof.rule {
                bounded(rule, 1024, "proof rule bytes")?;
            }
            for anchor in &proof.anchors {
                source(anchor)?;
            }
            if let Some(anchor) = &proof.wiring_site {
                source(anchor)?;
            }
            for candidate in &proof.candidates {
                bounded(&candidate.node_id, 512, "candidate identity bytes")?;
                bounded(&candidate.reason, 1024, "candidate reason bytes")?;
                if let Some(anchor) = &candidate.anchor {
                    source(anchor)?;
                }
            }
        }
        Ok(())
    }
    fn node_scalars(&mut self, node: &NodeRecord) -> std::result::Result<(), QueryError> {
        bounded(&node.id, 512, "node identity bytes")?;
        bounded(&node.name, 4096, "node name bytes")?;
        bounded(&node.qualified_name, 4096, "qualified name bytes")?;
        if let Some(anchor) = &node.source {
            source(anchor)?;
        }
        self.proofs(&node.evidence)
    }
    /// Signals inspect only this borrowed header. Ineligible metadata details
    /// are not copied solely to compute name-navigation hints.
    pub(crate) fn node_header(&mut self, node: &NodeRecord) -> std::result::Result<(), QueryError> {
        self.node_scalars(node)?;
        self.inspect(&(
            &node.id,
            &node.name,
            &node.qualified_name,
            &node.source,
            &node.evidence,
        ))
        .map_err(mapped)
    }
    /// Charge complete records, including fields not used by navigation, before
    /// a materialized record or its full response projection is cloned.
    pub(crate) fn node(&mut self, node: &NodeRecord) -> std::result::Result<(), QueryError> {
        self.node_scalars(node)?;
        self.inspect(node).map_err(mapped)
    }
    pub(crate) fn edge(&mut self, edge: &EdgeRecord) -> std::result::Result<(), QueryError> {
        bounded(&edge.id, 512, "edge identity bytes")?;
        bounded(&edge.key, 512, "edge key bytes")?;
        bounded(&edge.source, 512, "edge source identity bytes")?;
        bounded(&edge.target, 512, "edge target identity bytes")?;
        if let Some(anchor) = &edge.relationship_site {
            source(anchor)?;
        }
        if let Some(rule) = &edge.occurrence_rule {
            bounded(rule.as_str(), 1024, "occurrence rule bytes")?;
        }
        self.proofs(&edge.evidence)?;
        self.inspect(edge).map_err(mapped)
    }
    pub(crate) fn projection<T: ?Sized + Serialize>(
        &mut self,
        projection: &T,
    ) -> std::result::Result<(), QueryError> {
        self.inspect(projection).map_err(mapped)
    }
}

struct Inspector<'a> {
    budget: &'a mut RecordBudget,
    depth: usize,
}
impl Inspector<'_> {
    fn value(&mut self) -> Result<()> {
        if self.depth > self.budget.depth {
            return Err(InputError::Limit {
                resource: "metadata nesting depth",
                limit: MAX_DEPTH,
            });
        }
        self.budget.values = self.budget.values.checked_sub(1).ok_or(InputError::Limit {
            resource: "metadata values",
            limit: MAX_VALUES,
        })?;
        Ok(())
    }
    fn compound(mut self, len: Option<usize>) -> Result<Self> {
        self.value()?;
        if len.is_some_and(|len| len > self.budget.values) {
            return Err(InputError::Limit {
                resource: "metadata values",
                limit: MAX_VALUES,
            });
        }
        self.depth += 1;
        Ok(self)
    }
    fn child<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        value.serialize(Inspector {
            budget: self.budget,
            depth: self.depth,
        })
    }
    fn field<T: ?Sized + Serialize>(&mut self, key: &str, value: &T) -> Result<()> {
        self.budget.text(key.len())?;
        self.child(value)
    }
}

macro_rules! scalars {
    ($($name:ident($ty:ty)),* $(,)?) => {$(
        fn $name(mut self, _value: $ty) -> Result<()> { self.value() }
    )*};
}
impl ser::Serializer for Inspector<'_> {
    type Ok = ();
    type Error = InputError;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    scalars!(
        serialize_bool(bool),
        serialize_i8(i8),
        serialize_i16(i16),
        serialize_i32(i32),
        serialize_i64(i64),
        serialize_i128(i128),
        serialize_u8(u8),
        serialize_u16(u16),
        serialize_u32(u32),
        serialize_u64(u64),
        serialize_u128(u128),
        serialize_f32(f32),
        serialize_f64(f64)
    );
    fn serialize_char(mut self, value: char) -> Result<()> {
        self.value()?;
        self.budget.text(value.len_utf8())
    }
    fn serialize_str(mut self, value: &str) -> Result<()> {
        self.value()?;
        self.budget.text(value.len())
    }
    fn serialize_bytes(mut self, value: &[u8]) -> Result<()> {
        self.value()?;
        self.budget.text(value.len())
    }
    fn serialize_none(mut self) -> Result<()> {
        self.value()
    }
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<()> {
        self.compound(Some(1))?.child(value)
    }
    fn serialize_unit(mut self) -> Result<()> {
        self.value()
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<()> {
        self.serialize_unit()
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<()> {
        self.serialize_str(variant)
    }
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<()> {
        self.serialize_some(value)
    }
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<()> {
        self.budget.text(variant.len())?;
        self.serialize_some(value)
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<Self> {
        self.compound(len)
    }
    fn serialize_tuple(self, len: usize) -> Result<Self> {
        self.compound(Some(len))
    }
    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<Self> {
        self.compound(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self> {
        self.budget.text(variant.len())?;
        self.compound(Some(len))
    }
    fn serialize_map(self, len: Option<usize>) -> Result<Self> {
        self.compound(len.and_then(|len| len.checked_mul(2)))
    }
    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<Self> {
        self.compound(Some(len))
    }
    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self> {
        self.budget.text(variant.len())?;
        self.compound(Some(len))
    }
    // No formatting fallback may allocate an unchecked string.
    fn collect_str<T: ?Sized + std::fmt::Display>(self, _value: &T) -> Result<()> {
        Err(InputError::Invalid)
    }
}

macro_rules! sequences {
    ($($trait:ident::$method:ident),* $(,)?) => {$(
        impl $trait for Inspector<'_> {
            type Ok = (); type Error = InputError;
            fn $method<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> { self.child(value) }
            fn end(self) -> Result<()> { Ok(()) }
        }
    )*};
}
sequences!(
    SerializeSeq::serialize_element,
    SerializeTuple::serialize_element,
    SerializeTupleStruct::serialize_field,
    SerializeTupleVariant::serialize_field
);
impl SerializeMap for Inspector<'_> {
    type Ok = ();
    type Error = InputError;
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<()> {
        self.child(key)
    }
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.child(value)
    }
    fn end(self) -> Result<()> {
        Ok(())
    }
}
impl SerializeStruct for Inspector<'_> {
    type Ok = ();
    type Error = InputError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        self.field(key, value)
    }
    fn end(self) -> Result<()> {
        Ok(())
    }
}
impl SerializeStructVariant for Inspector<'_> {
    type Ok = ();
    type Error = InputError;
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        self.field(key, value)
    }
    fn end(self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    fn records() -> std::result::Result<(NodeRecord, EdgeRecord), Box<dyn Error>> {
        let node = serde_json::from_value(serde_json::json!({
            "id":"n", "kind":"function", "name":"n", "qualifiedName":"n", "evidence":[],
            "details":{"type":"symbol", "data":{"signature":""}}
        }))?;
        let edge = serde_json::from_value(serde_json::json!({
            "id":"e", "key":"e", "source":"n", "target":"m", "kind":"calls", "evidence":[], "context":""
        }))?;
        Ok((node, edge))
    }

    #[test]
    fn full_node_and_edge_metadata_share_exact_utf8_limit()
    -> std::result::Result<(), Box<dyn Error>> {
        let (mut node, mut edge) = records()?;
        let mut measured = RecordBudget::new();
        measured.node(&node)?;
        measured.edge(&edge)?;
        let remaining = measured.text;
        let Some(compass_model::code_graph::NodeDetails::Symbol(details)) = &mut node.details
        else {
            return Err("details".into());
        };
        let padding = "é".repeat(remaining / 4);
        let rest = remaining - padding.len();
        details.signature = Some(padding);
        edge.context = Some("x".repeat(rest));
        let mut budget = RecordBudget::new();
        budget.node(&node)?;
        budget.edge(&edge)?;
        assert_eq!(budget.text, 0);
        edge.context.as_mut().ok_or("context")?.push('x');
        let mut budget = RecordBudget::new();
        budget.node(&node)?;
        let error = budget
            .edge(&edge)
            .err()
            .ok_or("plus-one metadata accepted")?;
        assert_eq!(error.code(), "linked_record_metadata_limit");
        assert!(error.message().contains("metadata text bytes"));
        Ok(())
    }

    #[test]
    fn values_and_depth_are_checked_before_entering_children()
    -> std::result::Result<(), Box<dyn Error>> {
        use std::cell::Cell;
        struct Visited<'a>(&'a Cell<usize>);
        impl Serialize for Visited<'_> {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                self.0.set(self.0.get() + 1);
                serializer.serialize_unit()
            }
        }
        let visits = Cell::new(0);
        let children = (0..MAX_VALUES + 1)
            .map(|_| Visited(&visits))
            .collect::<Vec<_>>();
        assert!(RecordBudget::new().inspect(&children).is_err());
        assert_eq!(visits.get(), 0, "oversized vectors must not enter children");
        let mut nested = serde_json::Value::Null;
        for _ in 0..MAX_DEPTH {
            nested = serde_json::Value::Array(vec![nested]);
        }
        RecordBudget::new().inspect(&nested)?;
        nested = serde_json::Value::Array(vec![nested]);
        assert!(RecordBudget::new().inspect(&nested).is_err());
        Ok(())
    }

    #[test]
    fn source_paths_are_bounded_before_anchor_validation() -> std::result::Result<(), Box<dyn Error>>
    {
        let (mut node, mut edge) = records()?;
        let anchor = compass_model::provenance::SourceAnchor {
            file: "x".repeat(4096),
            start_byte: 0,
            end_byte: 1,
            start_line: 1,
            start_column: 0,
            end_line: 1,
            end_column: 1,
        };
        node.source = Some(anchor.clone());
        edge.relationship_site = Some(anchor);
        let mut budget = RecordBudget::new();
        budget.node(&node)?;
        budget.edge(&edge)?;
        node.source.as_mut().ok_or("node source")?.file.push('x');
        edge.relationship_site
            .as_mut()
            .ok_or("edge site")?
            .file
            .push('x');
        assert_eq!(
            RecordBudget::new()
                .node(&node)
                .err()
                .ok_or("node path accepted")?
                .code(),
            "linked_record_metadata_limit"
        );
        assert_eq!(
            RecordBudget::new()
                .edge(&edge)
                .err()
                .ok_or("edge path accepted")?
                .code(),
            "linked_record_metadata_limit"
        );
        Ok(())
    }
}
