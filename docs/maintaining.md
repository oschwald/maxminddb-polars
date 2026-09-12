# Maintainer operations

This guide covers repository settings and publishing responsibilities that are
not fully expressed in source files. Recheck external settings before a public
release and after ownership or publishing changes.

## Sources of truth

- [PyPI](https://pypi.org/project/maxminddb-polars/) lists published Python
  packages, requirements, and distributions.
- [`pyproject.toml`](../pyproject.toml) and [`Cargo.toml`](../Cargo.toml) declare
  package metadata and dependencies; `Cargo.lock`, `fuzz/Cargo.lock`, and
  `uv.lock` pin the resolved development dependencies.
- [`rust-toolchain.toml`](../rust-toolchain.toml) selects the repository Rust
  toolchain.
- The [compatibility](../.github/workflows/compatibility.yml) and
  [artifact](../.github/workflows/artifacts.yml) workflows define the tested
  Python/Polars combinations and distribution platforms.
- The `tests/data` submodule pins the MaxMind-DB fixtures. `scripts/check` and
  the metadata CI job verify that revision.
- [`CONTRIBUTING.md`](../CONTRIBUTING.md#releases) describes the release helper.

## GitHub settings

The following repository settings were verified on 2026-09-12:

- The repository is public, `main` is the default branch, and Issues are enabled.
- Dependency graph, Dependabot security updates, private vulnerability
  reporting, secret scanning, and push protection are enabled.
- Actions defaults to read-only permissions and cannot approve pull requests.
- `main` requires a pull request, one approving review, resolved conversations,
  and an up-to-date branch. New commits dismiss stale approvals.
- `main` blocks force pushes and deletion. Administrators retain a recovery
  bypass for its required reviews and checks.
- The `Protect release tags` ruleset blocks force updates and deletion of `v*`
  tags, with no bypass actors.
- Merge, squash, and rebase merges are allowed; merged branches are deleted.

The required status checks on `main` are:

- `Clippy`
- `Metadata and locks`
- `Prettier`
- `Rustfmt`
- `Rust unit tests`
- `Python 3.10 on ubuntu-latest`

These are literal check names in branch protection. If the CI job names or
Python matrix change, update branch protection to match.
`Validate distributions` is intentionally not required because the artifact
workflow is path-filtered; making it required would block pull requests for
which it does not run.

Code scanning, dependency review, auditing, and artifact retention are
configured in [the workflows](../.github/workflows). Check the relevant runs
for the release candidate instead of relying on an earlier successful run.

## Publishing

A published GitHub release starts [`release.yml`](../.github/workflows/release.yml).
It checks the tag against the Cargo version, builds and validates distributions,
publishes to crates.io and PyPI, and verifies the published packages. A manual
workflow dispatch validates artifacts without publishing.

Both registry projects are established. Their trusted publishers use this
repository's owner and name, the `release.yml` workflow, and these environments:

| Registry  | GitHub environment |
| --------- | ------------------ |
| crates.io | `release`          |
| PyPI      | `pypi`             |

Both environments accept only `v*` tags and have no stored secrets, as verified
on 2026-09-12. Registry uploads use short-lived OIDC credentials. Repository
transfers, workflow renames, or environment renames require corresponding
trusted-publisher changes at both registries.

Maintainers must keep account two-factor authentication and recovery access
available; those account settings cannot be verified through the repository API.

## Temporary security exceptions

The authoritative advisory ignore list and re-evaluation deadline live in
[`.cargo/audit.toml`](../.cargo/audit.toml). The current rationale is:

- [`bincode` is unmaintained](https://rustsec.org/advisories/RUSTSEC-2025-0141.html)
  and is required transitively by Polars.
- The [`quick-xml` attribute-checking](https://rustsec.org/advisories/RUSTSEC-2026-0194.html)
  and [namespace-allocation](https://rustsec.org/advisories/RUSTSEC-2026-0195.html)
  advisories concern a dependency locked through disabled Polars cloud features.
  It is absent from the configured build targets, and the plugin does not parse
  XML. Recheck that assumption for both workspaces when changing dependencies:

  ```console
  cargo tree --locked --target all -i quick-xml
  cargo tree --locked --manifest-path fuzz/Cargo.toml --target all -i quick-xml
  ```

Re-evaluate exceptions when the Polars dependency stack changes and before the
deadline in the audit configuration. Historical resolved advisories belong in
the release record, not the active exception list.
