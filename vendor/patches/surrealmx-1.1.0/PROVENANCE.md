# SurrealMX recovery codec dependency

This source export is from the published SurrealMX 1.1.0 archive, Git
revision 8e9810c0e3d41fde72394384234c1f9d0497e180 and SHA-256
a5ed0da12b19262916a8ddb7ac8a340772113ce91b8c845aa7bc994b69d39715.
UAR resolves the dependency through its workspace lockfile.

The manifests keep the bincode API alias and select grovedb-bincode 2.1.0.
The fork documents compatibility with the existing wire format and ordinary
Serde APIs. The SurrealMX record schema, codec configuration, and error
handling are unchanged. Qualification must exercise existing append-only
log recovery and cross-version reopen against disposable data before
making a compatibility claim.

Sources
- https://rustsec.org/advisories/RUSTSEC-2025-0141.html
- https://docs.rs/crate/grovedb-bincode/2.1.0
