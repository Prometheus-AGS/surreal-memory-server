# Container publication: approved local artifacts

Container compilation, tests, checks and release certification run on the local
development machine. GitHub Actions must not build or validate memory-server
images on pushes, pull requests, tags or manual dispatch. The former
`.github/workflows/image.yml` is removed from the current source; Git retains
its historical definition, without another runnable archived workflow.

For an approved container release, complete the coherent production source and
applicable local integration gates first. Build each intended platform image
locally using the approved source, lockfile and feature/dependency selection,
then certify the actual production entry point and collaborating services for
that artifact. Record source revision, platform, build inputs, artifact digest
and real local gate results. A compiled image, tag, source merge or uploaded
artifact alone cannot establish functional acceptance.

Publish only those already built and certified artifacts after the required
owner approvals. Use the explicitly selected registry/destination and approved
tags; uploading or assembling a manifest must not rebuild an image or rerun
validation on a hosted runner. Preserve each published digest and its matching
local receipts. A multi-platform claim requires approved evidence for every
included platform; do not infer another architecture's acceptance from one local
build. Keep the previous approved digest for rollback without replacing newer
runtime data or inventing recovery evidence.

This source correction introduces no publisher infrastructure and does not
publish, tag, push or certify an artifact. Exact release metadata, protected pin
approvals and registry authorization remain with the release owner. Historical
images or earlier hosted runs are not evidence for the current release candidate.

The only permitted hosted operations are deterministic documentation sync and
GitHub Pages packaging/deployment. `.github/workflows/docs.yml` packages the
Docusaurus site and deploys its Pages artifact; it performs no separate hosted
type-check, test, lint, doctor or contract/certification gate. These steps publish
documentation and cannot substitute for local validation.
