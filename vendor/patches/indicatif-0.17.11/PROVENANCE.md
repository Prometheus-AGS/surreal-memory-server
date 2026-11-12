# Indicatif unit prefix backport

Base is the published indicatif 0.17.11 crate. The registry archive
SHA-256 is 183b3088984b400f4cfac3620d5e076c84da5364016b4f49473de574b2586235.
The release archive records fe5900305c80c7210ea7d1029b7cd47cac608d30 and marks
its VCS state dirty. The registry bytes identify the source used here.

This backports the NumberPrefix import and dependency used in the
indicatif 0.18.3 release at 23100f5e52d679af7c64ee283af5ea0e08742393.
Formatting expressions, public types, features, examples, tests, and license
are preserved. The UAR graph already resolves unit-prefix 0.5.2.

Sources
- https://rustsec.org/advisories/RUSTSEC-2025-0119.html
- https://docs.rs/crate/indicatif/0.18.3/source/src/format.rs
