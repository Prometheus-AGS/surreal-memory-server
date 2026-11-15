//! Paged collection of what a persona mindmap renders (issue #62).
//!
//! Shared by the MCP and REST persona generators. Memories are read through
//! bounded pages without embeddings, and only the per-category state the
//! mindmap shows is kept: the first few items, truncated, plus a count.

use std::collections::BTreeMap;

use anyhow::Result;
use surreal_memory::{MemoryPageRequest, MemoryStorage};

/// Leaves rendered per category branch.
pub const LEAVES_PER_CATEGORY: usize = 5;
/// Leaf labels longer than this many bytes are truncated.
const LEAF_LABEL_BYTES: usize = 80;
/// Category used for memories that have none.
const DEFAULT_CATEGORY: &str = "general";

/// One category branch of a persona mindmap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaCategory {
    pub name: String,
    /// The first [`LEAVES_PER_CATEGORY`] memories' labels, truncated.
    pub leaves: Vec<String>,
    /// Memories read in this category.
    pub total: usize,
}

/// Everything a persona mindmap is built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonaClusters {
    /// Categories in name order.
    pub categories: Vec<PersonaCategory>,
    /// Memories read in all categories.
    pub total: usize,
}

/// `content`, cut to at most [`LEAF_LABEL_BYTES`] bytes on a character
/// boundary, with an ellipsis when it was cut.
fn leaf_label(content: &str) -> String {
    if content.len() <= LEAF_LABEL_BYTES {
        return content.to_string();
    }
    let end = (0..=LEAF_LABEL_BYTES)
        .rev()
        .find(|&index| content.is_char_boundary(index))
        .unwrap_or(0);
    format!("{}…", &content[..end])
}

/// Read `user_id`'s live memories page by page and cluster them by their first
/// category. A failed page fails the whole collection.
pub async fn collect_persona(
    storage: &dyn MemoryStorage,
    user_id: &str,
) -> Result<PersonaClusters> {
    let mut clusters: BTreeMap<String, PersonaCategory> = BTreeMap::new();
    let mut total = 0usize;
    let mut request = MemoryPageRequest {
        user_id: Some(user_id.to_string()),
        ..MemoryPageRequest::default()
    };
    loop {
        let page = storage.list_memories_page(request.clone()).await?;
        for memory in &page.memories {
            let name = memory
                .categories
                .first()
                .cloned()
                .unwrap_or_else(|| DEFAULT_CATEGORY.to_string());
            let category = clusters
                .entry(name.clone())
                .or_insert_with(|| PersonaCategory {
                    name,
                    leaves: Vec::new(),
                    total: 0,
                });
            if category.leaves.len() < LEAVES_PER_CATEGORY {
                category.leaves.push(leaf_label(&memory.content));
            }
            category.total += 1;
            total += 1;
        }
        match page.next_cursor {
            Some(cursor) => request.cursor = Some(cursor),
            None => break,
        }
    }
    Ok(PersonaClusters {
        categories: clusters.into_values().collect(),
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_labels_are_cut_on_a_character_boundary() {
        // 27 three-byte characters: byte 80 falls inside the 27th.
        let euros = "€".repeat(27);
        assert_eq!(leaf_label(&euros), format!("{}…", "€".repeat(26)));
    }

    #[test]
    fn short_and_ascii_labels_keep_their_previous_form() {
        assert_eq!(leaf_label("short"), "short");
        let ascii = "a".repeat(81);
        assert_eq!(leaf_label(&ascii), format!("{}…", "a".repeat(80)));
    }
}
