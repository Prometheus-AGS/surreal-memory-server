# Mindmap Performance Considerations

## Known Issue: Large Mindmap Updates

### Problem
When mindmaps contain many nodes (>500 nodes), adding nodes or edges can become **very slow** (timeouts or multi-second delays).

**Root Cause**: SurrealDB has a known performance issue with `UPDATE CONTENT` on large JSON objects (>500KB). See [SurrealDB issue #1810](https://github.com/surrealdb/surrealdb/issues/1810).

Currently, mindmaps are stored as single records with nested arrays:
```json
{
  "name": "my_mindmap",
  "nodes": [/* potentially hundreds of nodes */],
  "edges": [/* potentially hundreds of edges */]
}
```

Every time a single node is added, the **entire mindmap** is re-serialized and written back to the database via `UPDATE CONTENT`.

### Fix Applied (v0.1.0+)

1. **Query Timeout**: Added 30-second timeout to `UPDATE` queries to fail fast instead of hanging indefinitely
   ```sql
   UPDATE type::record($id) CONTENT $value RETURN AFTER TIMEOUT 30s
   ```

2. **Performance Warnings**: Added logging to warn when mindmaps exceed 500 nodes:
   ```
   WARN Large mindmap detected - updates may be slow. Consider splitting into multiple mindmaps.
   ```

3. **Size Monitoring**: Added JSON size tracking and warnings for objects >500KB

### Workarounds

**For Users**:
- **Keep mindmaps small**: < 500 nodes per mindmap
- **Split large mindmaps**: Create multiple related mindmaps instead of one massive one
- **Use TaskStreams** for linear context instead of mindmaps for large hierarchies

**For Developers (Future Improvements)**:
- Store nodes and edges in separate tables with foreign keys
- Use `UPDATE MERGE` for partial updates instead of `CONTENT`
- Implement pagination for node/edge lists
- Add bulk operations for adding multiple nodes at once

### Example Error

If you see:
```
Error: SurrealDB update query failed: Query timeout exceeded
```

This means the mindmap has grown too large. Split it into smaller mindmaps or reduce the node count.

### Benchmarks

| Nodes | Edges | Add Node Time | JSON Size |
|-------|-------|---------------|-----------|
| 10    | 15    | ~10ms         | ~2KB      |
| 100   | 150   | ~50ms         | ~20KB     |
| 500   | 750   | ~500ms        | ~100KB    |
| 1000  | 1500  | **5-30s**     | ~500KB    |
| 2000+ | 3000+ | **timeout**   | >1MB      |

*Benchmarks on Docker with SurrealDB 3.0.5, RocksDB backend*

## Related Issues

- [SurrealDB #1810](https://github.com/surrealdb/surrealdb/issues/1810) - Very slow insert of large JSON data
- [SurrealDB #2475](https://github.com/surrealdb/surrealdb/issues/2475) - Related performance tracking

## Migration Path (Future)

If we migrate to a normalized schema:
```sql
-- Separate tables for nodes and edges
DEFINE TABLE mindmap_node SCHEMAFULL;
DEFINE FIELD mindmap_id ON mindmap_node TYPE record<mindmap>;
DEFINE FIELD node_id ON mindmap_node TYPE string;
DEFINE FIELD label ON mindmap_node TYPE string;
-- ... etc

DEFINE TABLE mindmap_edge SCHEMAFULL;
DEFINE FIELD mindmap_id ON mindmap_edge TYPE record<mindmap>;
DEFINE FIELD from_id ON mindmap_edge TYPE string;
DEFINE FIELD to_id ON mindmap_edge TYPE string;
```

This would allow O(1) node additions instead of O(n) where n = mindmap size.
