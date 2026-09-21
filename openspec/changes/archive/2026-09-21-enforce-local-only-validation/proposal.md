## Why

Pushing PR #22 started a hosted `Build, Clippy & Test` job even though this
repository requires product validation to run locally. Leaving that workflow in
place wastes hosted capacity and can misrepresent GitHub results as release
evidence.

## What Changes

- Remove the hosted Rust build, Clippy, database-fixture, and test workflow.
- Keep the documentation deployment workflow, whose hosted work is limited to
  packaging and publishing the documentation site.

## Capabilities

### New Capabilities

- `local-validation-policy`: Defines where product validation and documentation deployment execute.

### Modified Capabilities

None.

## Impact

The change removes `.github/workflows/ci.yml`. Developers and release tooling
continue to run Rust validation locally; documentation deployment remains on
GitHub Pages.
