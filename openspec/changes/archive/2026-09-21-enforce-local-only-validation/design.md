## Context

The repository already requires builds, tests, formatting, linting, and release
certification to run on the development machine. A legacy pull-request workflow
still ran those checks on GitHub Actions and was observed starting on PR #22.

## Goals / Non-Goals

**Goals:**

- Remove the observed hosted product-validation path.
- Preserve documentation packaging and GitHub Pages deployment.

**Non-Goals:**

- Move product tests to another hosted runner.
- Change local Rust validation commands.
- Change documentation content or deployment behavior.

## Decisions

Delete the product CI workflow rather than disabling individual jobs. Every job
in that file performs prohibited product validation, so no deployment behavior
would remain after filtering it.

The documentation workflow remains because its artifact build and publication
are the deployment mechanism. Its type check protects the deployable
documentation artifact and is not used as product certification.

## Risks / Trade-offs

- GitHub will no longer display Rust test checks on pull requests. Local command
  evidence in the change record and PR description is authoritative.
- Existing in-flight runs must be canceled separately; deleting the workflow
  prevents later pushes from starting new runs.

## Migration Plan

Cancel the observed PR #22 run, delete the CI workflow, validate the OpenSpec
change locally, and merge the removal through a dedicated pull request.

## Open Questions

None.
