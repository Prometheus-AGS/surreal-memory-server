//! Mindmap — structured visual knowledge representation for persona modeling and ideation.
//!
//! Supports 5 map types derived from established cognitive frameworks:
//! - `Radial` (Tony Buzan, 1974) — association radiating from a central concept
//! - `Concept` (Novak, 1972) — labeled propositional links between concepts
//! - `Argument` — claim/evidence/rebuttal sensemaking maps
//! - `Tree` — hierarchical decomposition (org charts, capability trees)
//! - `Temporal` — concept evolution across time periods

use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use surrealdb::types::{Datetime, RecordId};
use surrealdb_types::SurrealValue;

/// The structural type of a mindmap — determines rendering and semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, SurrealValue, Default)]
#[serde(rename_all = "lowercase")]
pub enum MapType {
    /// Tony Buzan radial — central node + radiating branches.
    #[default]
    Radial,
    /// Novak concept map — labeled directed edges (e.g. "leads to", "requires").
    Concept,
    /// Argument / deliberation map — claim, evidence, rebuttal nodes.
    Argument,
    /// Hierarchical tree decomposition (non-radial parent→children).
    Tree,
    /// Temporal / timeline — branches represent time periods.
    Temporal,
}

impl MapType {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Radial => "radial",
            Self::Concept => "concept",
            Self::Argument => "argument",
            Self::Tree => "tree",
            Self::Temporal => "temporal",
        }
    }

    pub fn parse_str(raw: &str) -> Result<Self, String> {
        match raw {
            "radial" => Ok(Self::Radial),
            "concept" => Ok(Self::Concept),
            "argument" => Ok(Self::Argument),
            "tree" => Ok(Self::Tree),
            "temporal" => Ok(Self::Temporal),
            _ => Err(format!("unknown map_type '{raw}'")),
        }
    }
}

impl fmt::Display for MapType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for MapType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_str(s)
    }
}

/// A node within a mindmap.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Default)]
pub struct MindMapNode {
    /// Unique ID within this mindmap (not a SurrealDB record ID).
    pub id: String,
    /// Display label for the node.
    pub label: String,
    /// Parent node ID (None = root).
    pub parent_id: Option<String>,
    /// Node type for argument maps: `claim | evidence | rebuttal | idea`.
    pub node_type: Option<String>,
    /// Optional hex color (e.g. `"#4a90e2"`).
    pub color: Option<String>,
    /// Arbitrary JSON metadata.
    pub metadata: Option<serde_json::Value>,
}

/// A directed or undirected edge between two nodes.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue, Default)]
pub struct MindMapEdge {
    pub from_id: String,
    pub to_id: String,
    /// Optional relationship label (e.g. `"supports"`, `"contradicts"`, `"leads to"`).
    pub label: Option<String>,
    /// True for concept maps and argument maps.
    pub directed: bool,
}

/// A mindmap entity — the top-level container persisted in SurrealDB.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct MindMap {
    pub id: Option<RecordId>,
    pub name: String,
    pub description: Option<String>,
    pub map_type: MapType,
    /// Agent that owns/created this mindmap.
    pub agent_id: Option<String>,
    /// User this mindmap belongs to (for persona maps).
    pub user_id: Option<String>,
    /// Optional TaskStream association.
    pub task_stream_id: Option<RecordId>,
    /// Taxonomy tags for discovery.
    #[serde(default)]
    pub tags: Vec<String>,
    /// All nodes in this mindmap.
    #[serde(default)]
    pub nodes: Vec<MindMapNode>,
    /// All edges in this mindmap.
    #[serde(default)]
    pub edges: Vec<MindMapEdge>,
    pub created_at: Datetime,
    pub updated_at: Datetime,
}

impl MindMap {
    /// Create a new mindmap with a root node.
    pub fn new(
        name: impl Into<String>,
        map_type: MapType,
        root_label: impl Into<String>,
        description: Option<String>,
        agent_id: Option<String>,
        user_id: Option<String>,
    ) -> Self {
        let now = Datetime::default();
        let root_label = root_label.into();
        let root_node = MindMapNode {
            id: "root".to_string(),
            label: root_label,
            parent_id: None,
            node_type: None,
            color: Some("#4a90e2".to_string()),
            metadata: None,
        };
        Self {
            id: None,
            name: name.into(),
            map_type,
            description,
            agent_id,
            user_id,
            task_stream_id: None,
            tags: vec![],
            nodes: vec![root_node],
            edges: vec![],
            created_at: now,
            updated_at: now,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::MapType;

    #[test]
    fn map_type_codec_round_trips() {
        for map_type in [
            MapType::Radial,
            MapType::Concept,
            MapType::Argument,
            MapType::Tree,
            MapType::Temporal,
        ] {
            assert_eq!(MapType::parse_str(map_type.as_str()).unwrap(), map_type);
        }
    }

    #[test]
    fn map_type_codec_rejects_unknown_values() {
        assert!(MapType::parse_str("Radial").is_err());
        assert!(MapType::parse_str("unknown").is_err());
    }
}

/// Export format for `export_mindmap`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Mermaid,
    Markdown,
}

impl MindMap {
    /// Export this mindmap to the specified format.
    pub fn export(&self, format: &ExportFormat) -> String {
        match format {
            ExportFormat::Json => serde_json::to_string_pretty(self).unwrap_or_default(),
            ExportFormat::Mermaid => self.to_mermaid(),
            ExportFormat::Markdown => self.to_markdown(),
        }
    }

    /// Validate structural integrity of the mindmap.
    ///
    /// Checks performed:
    /// - No duplicate node IDs.
    /// - Every `parent_id` reference points to an existing node.
    /// - Every edge `from_id` / `to_id` references an existing node.
    /// - Radial and Tree maps have exactly one root node (node without a `parent_id`).
    pub fn validate(&self) -> anyhow::Result<()> {
        use std::collections::HashSet;

        // Collect all node IDs; detect duplicates in one pass.
        let mut seen: HashSet<&str> = HashSet::new();
        for node in &self.nodes {
            if !seen.insert(node.id.as_str()) {
                anyhow::bail!("Duplicate node id '{}' in mindmap '{}'", node.id, self.name);
            }
        }

        // Validate parent references.
        for node in &self.nodes {
            if let Some(parent_id) = &node.parent_id {
                anyhow::ensure!(
                    seen.contains(parent_id.as_str()),
                    "Node '{}' references unknown parent '{}' in mindmap '{}'",
                    node.id,
                    parent_id,
                    self.name
                );
            }
        }

        // Validate edge endpoints.
        for edge in &self.edges {
            if !seen.contains(edge.from_id.as_str()) {
                anyhow::bail!(
                    "Edge from_id '{}' references unknown node in mindmap '{}'",
                    edge.from_id,
                    self.name
                );
            }
            if !seen.contains(edge.to_id.as_str()) {
                anyhow::bail!(
                    "Edge to_id '{}' references unknown node in mindmap '{}'",
                    edge.to_id,
                    self.name
                );
            }
        }

        // Tree and Radial maps must have exactly one root.
        if matches!(self.map_type, MapType::Radial | MapType::Tree) {
            let root_count = self.nodes.iter().filter(|n| n.parent_id.is_none()).count();
            if root_count != 1 {
                anyhow::bail!(
                    "Mindmap '{}' (type {:?}) must have exactly 1 root node, found {}",
                    self.name,
                    self.map_type,
                    root_count
                );
            }
        }

        Ok(())
    }

    /// Compute the depth of a node by walking the `parent_id` chain.
    ///
    /// Returns 0 for root nodes.  Cycles are broken after visiting more nodes
    /// than exist in the map.
    fn depth_of(&self, node_id: &str) -> usize {
        let mut depth = 0usize;
        let mut current = node_id;
        // Safety cap: stop after visiting more hops than there are nodes.
        let limit = self.nodes.len();
        while depth < limit {
            match self.nodes.iter().find(|n| n.id == current) {
                Some(n) => match &n.parent_id {
                    Some(parent) => {
                        depth += 1;
                        current = parent.as_str();
                    }
                    None => break,
                },
                None => break,
            }
        }
        depth
    }

    fn to_mermaid(&self) -> String {
        use std::collections::HashSet;

        let mut out = String::from("graph TD\n");

        // Emit node declarations.
        for node in &self.nodes {
            let safe_id = node.id.replace([':', '-', '.', ' '], "_");
            out.push_str(&format!("    {}[\"{}\"]\n", safe_id, node.label));
        }

        // Track emitted (from, to) pairs to avoid duplicates between explicit
        // edges and the implicit parent→child edges derived from node.parent_id.
        let mut emitted: HashSet<(String, String)> = HashSet::new();

        // Emit explicit edges first.
        for edge in &self.edges {
            let from = edge.from_id.replace([':', '-', '.', ' '], "_");
            let to = edge.to_id.replace([':', '-', '.', ' '], "_");
            if let Some(label) = &edge.label {
                out.push_str(&format!("    {} -->|{}| {}\n", from, label, to));
            } else {
                out.push_str(&format!("    {} --> {}\n", from, to));
            }
            emitted.insert((from, to));
        }

        // Emit implicit parent→child edges only when not already covered above.
        for node in &self.nodes {
            if let Some(parent_id) = &node.parent_id {
                let parent = parent_id.replace([':', '-', '.', ' '], "_");
                let child = node.id.replace([':', '-', '.', ' '], "_");
                if emitted.insert((parent.clone(), child.clone())) {
                    out.push_str(&format!("    {} --> {}\n", parent, child));
                }
            }
        }

        out
    }

    fn to_markdown(&self) -> String {
        let mut out = format!("# {}\n\n", self.name);
        if let Some(desc) = &self.description {
            out.push_str(&format!("{}\n\n", desc));
        }
        out.push_str(&format!("**Type:** {:?}\n\n", self.map_type));
        out.push_str("## Nodes\n\n");
        for node in &self.nodes {
            // Compute actual nesting depth by walking the parent_id chain.
            let depth = self.depth_of(&node.id);
            let indent = "  ".repeat(depth);
            out.push_str(&format!("{}- `{}` — {}\n", indent, node.id, node.label));
        }
        if !self.edges.is_empty() {
            out.push_str("\n## Edges\n\n");
            for edge in &self.edges {
                let arrow = if edge.directed { "→" } else { "—" };
                let label = edge.label.as_deref().unwrap_or("");
                out.push_str(&format!(
                    "- `{}` {} `{}` {}\n",
                    edge.from_id, arrow, edge.to_id, label
                ));
            }
        }
        out
    }
}
