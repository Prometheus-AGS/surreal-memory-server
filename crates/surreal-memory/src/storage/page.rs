//! Bounded, resumable memory listing (issue #62).
//!
//! One page request names the owner filters, a page size, an optional cursor
//! and whether embeddings are wanted. The limits below are enforced by the
//! storage method itself, so the library, MCP and REST cannot disagree.

use serde::{Deserialize, Serialize};

use crate::memory::Memory;

/// Page size used when a request does not name one.
pub const DEFAULT_MEMORY_PAGE_SIZE: usize = 50;
/// Largest page size a request may ask for without embeddings.
pub const MAX_MEMORY_PAGE_SIZE: usize = 200;
/// Largest page size a request may ask for with embeddings.
pub const MAX_MEMORY_PAGE_SIZE_WITH_EMBEDDINGS: usize = 25;
/// Largest encoded size (JSON bytes of the page's memories) a page is filled
/// to. A single memory larger than this is still returned alone.
pub const MEMORY_PAGE_BYTE_BUDGET: usize = 1024 * 1024;

/// One page of a memory listing.
///
/// Pages are ordered by record ID. Following `cursor` from page to page
/// returns every memory that exists for the whole iteration exactly once.
/// It is not a snapshot: a memory created during the iteration may be absent,
/// and a memory deleted or expired during it does not appear afterwards.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryPageRequest {
    pub user_id: Option<String>,
    pub agent_id: Option<String>,
    pub session_id: Option<String>,
    /// Page size; `None` means [`DEFAULT_MEMORY_PAGE_SIZE`]. Zero or above the
    /// applicable maximum is rejected, never clamped.
    pub limit: Option<usize>,
    /// The `next_cursor` of the previous page; `None` starts the listing.
    pub cursor: Option<String>,
    /// Return each memory's embedding. Subject to
    /// [`MAX_MEMORY_PAGE_SIZE_WITH_EMBEDDINGS`].
    pub include_embeddings: bool,
}

/// One page of memories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryPage {
    pub memories: Vec<Memory>,
    /// Pass back as `cursor` to fetch the next page; `None` when complete.
    pub next_cursor: Option<String>,
    /// True when more memories remain after this page.
    pub has_more: bool,
}
