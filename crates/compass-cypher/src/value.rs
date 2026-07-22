use std::collections::BTreeMap;
use std::sync::Arc;

use compass_model::{EdgeIndex, NodeIndex};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct NodeRef {
    pub index: NodeIndex,
    pub id: Arc<str>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct RelationshipRef {
    pub index: EdgeIndex,
    pub source: Arc<str>,
    pub target: Arc<str>,
    pub relation: Arc<str>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PathRef {
    pub nodes: Arc<[NodeRef]>,
    pub relationships: Arc<[RelationshipRef]>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CompassValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(Arc<str>),
    List(Arc<[CompassValue]>),
    Map(Arc<BTreeMap<String, CompassValue>>),
    Node(NodeRef),
    Relationship(RelationshipRef),
    Path(PathRef),
}

impl CompassValue {
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    #[must_use]
    pub const fn compass_type(&self) -> CompassType {
        match self {
            Self::Null => CompassType::Null,
            Self::Boolean(_) => CompassType::Boolean,
            Self::Integer(_) => CompassType::Integer,
            Self::Float(_) => CompassType::Float,
            Self::String(_) => CompassType::String,
            Self::List(_) => CompassType::List,
            Self::Map(_) => CompassType::Map,
            Self::Node(_) => CompassType::Node,
            Self::Relationship(_) => CompassType::Relationship,
            Self::Path(_) => CompassType::Path,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompassType {
    Any,
    Null,
    Boolean,
    Integer,
    Float,
    String,
    List,
    Map,
    Node,
    Relationship,
    Path,
}

pub type ParameterTypes = BTreeMap<String, CompassType>;
pub type Parameters = BTreeMap<String, CompassValue>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub value_type: CompassType,
    pub nullable: bool,
}

pub type Row = Vec<CompassValue>;
