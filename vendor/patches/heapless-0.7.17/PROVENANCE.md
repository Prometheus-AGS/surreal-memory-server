# Heapless 0.7.17 source overlay

This directory derives from the crates.io release of heapless 0.7.17. The
registry checksum is
`cdc6457c0eb62c71aac4bc17216026d8410337c4126773b9c5daba343f17964f`.
The release's MIT and Apache-2.0 license files are included.

The five embedded target dependency entries use portable-atomic with
critical-section, the maintained successor named by
[RUSTSEC-2023-0089](https://rustsec.org/advisories/RUSTSEC-2023-0089.html).
The dependency alias keeps the existing Cargo feature and source imports.
The existing optional dependencies and target cfgs retain their roles.
The application continues to provide its platform critical-section
implementation where the original fallback required one.
[Portable-atomic PR #51](https://github.com/taiki-e/portable-atomic/pull/51)
introduced this synchronization backend.

The build script declares its custom cfgs for Cargo's check-cfg support.
