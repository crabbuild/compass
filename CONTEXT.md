# Compass domain language

This glossary records Compass-specific concepts whose meaning must stay stable
across extraction, projection, output, and viewer modules.

## Language

**Architecture projection**:
A bounded, source-scoped, relationship-typed presentation of graph communities as project owners, subsystem groups, routes, omissions, and quality evidence. It never rewrites the underlying graph or treats display names as identity.
_Avoid_: Call-flow sections, architecture inference, subsystem catalog

**Source scope**:
The evidence-backed classification of a graph node as Production, Test, Generated, Vendor, or Unknown for one architecture projection.
_Avoid_: Visibility filter, hidden code

**Architecture lens**:
A declared subset of typed graph relationships used to explain one aspect of project topology, such as execution, dependency, type, or structure.
_Avoid_: Edge filter, call filter
