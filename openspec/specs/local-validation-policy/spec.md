# local-validation-policy Specification

## Purpose
Keep product verification on owned local machines while allowing hosted automation only for documentation deployment and release container-image publication.

## Requirements

### Requirement: Product validation runs locally

The project SHALL run builds, checks, linting, tests, database fixtures, and
release certification on a local development machine. GitHub Actions MUST NOT
run those product-validation commands.

#### Scenario: A source pull request is pushed

- **WHEN** a pull request changes Rust source, tests, or configuration
- **THEN** GitHub Actions does not start a product build, lint, or test job
- **AND** the pull request records the applicable local validation evidence

### Requirement: Hosted automation is limited to documentation deployment and release image publication

GitHub Actions SHALL be limited to packaging and deploying the documentation
site and to building and publishing the release container image. Checks that
are intrinsic to producing the deployable documentation artifact MAY run inside
that deployment workflow. The image workflow MUST NOT run linting, tests,
database fixtures, or release certification; those run locally before the
version tag that triggers it.

#### Scenario: Documentation deployment runs

- **WHEN** a qualifying documentation change reaches the deployment workflow
- **THEN** the workflow may package and publish the documentation site
- **AND** it does not run Rust product validation or release certification

#### Scenario: A version tag is pushed

- **WHEN** a `v*` tag, or a push to `main`, reaches the repository
- **THEN** the image workflow builds the container image and publishes it to GHCR
- **AND** it does not run Rust linting, tests, or release certification
