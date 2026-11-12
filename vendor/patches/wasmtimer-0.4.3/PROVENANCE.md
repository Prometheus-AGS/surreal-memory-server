# Wasmtimer pin projection

The base is wasmtimer 0.4.3, registry commit
fecc90e04b1884ac6298af2cdcd2ef3f8e0141de and crate checksum
1c598d6b99ea013e35844697fc4670d08339d5cda15588f193c6beedd12f644b.
RUSTSEC-2026-0441 recommends pin-project-lite as a replacement for the
unmaintained unsafe projection macros. The root lock already selects
pin-project-lite 0.2.17.

Interval already requires an Unpin Sleep for reset. Poll that field
directly. Timeout projects both pinned fields through pin_project! and
preserves ordering and the public API, including support for !Unpin
futures. This overlay retains the published manifest, implementation and
browser tests with only the dependency and projection edits above. The
archive's standalone workspaces retain their own lockfiles.

Source: https://rustsec.org/advisories/RUSTSEC-2026-0441.html
