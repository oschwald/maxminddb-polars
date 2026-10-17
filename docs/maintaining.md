# Maintainer operations

Recheck repository and publisher settings before a release and after ownership
or publishing changes.

## Sources of truth

- [PyPI](https://pypi.org/project/maxminddb-polars/) lists published packages.
- [`pyproject.toml`](../pyproject.toml) and [`Cargo.toml`](../Cargo.toml) declare
  versions and dependencies. `Cargo.lock`, `fuzz/Cargo.lock`, and `uv.lock` pin
  development dependencies.
- [`rust-toolchain.toml`](../rust-toolchain.toml) selects the repository Rust
  toolchain.
- The [compatibility](../.github/workflows/compatibility.yml) and
  [artifact](../.github/workflows/artifacts.yml) workflows define the tested
  Python/Polars versions and platforms.
- `tests/data` pins the fixtures. `scripts/check` and metadata CI verify the revision.
- [`CONTRIBUTING.md`](../CONTRIBUTING.md#releases) describes the release helper.

## GitHub settings

Settings verified on 2026-09-12:

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
- Merge, squash, and rebase merges are allowed. Merged branches are deleted.

The required status checks on `main` are:

- `Clippy`
- `Metadata and locks`
- `Prettier`
- `Rustfmt`
- `Rust unit tests`
- `Python 3.10 on ubuntu-latest`

Keep branch protection's check names in sync with CI. Do not require
`Validate distributions`: its workflow skips some pull requests based on changed
paths, which would leave those requests blocked.

Before releasing, check the candidate's workflow results. The
[workflows](../.github/workflows) define code scanning, dependency review,
audits, and artifact retention.

## Publishing

Publishing a GitHub release starts [`release.yml`](../.github/workflows/release.yml).
It checks the tag against the Cargo version, builds and validates packages,
publishes them to crates.io and PyPI, and verifies the uploads. A manual run
validates without publishing.

Both registries use trusted publishing with this repository's owner and name,
the `release.yml` workflow, and these environments:

| Registry  | GitHub environment |
| --------- | ------------------ |
| crates.io | `release`          |
| PyPI      | `pypi`             |

Both environments accept only `v*` tags and have no stored secrets, as verified
on 2026-09-12. Uploads use short-lived OpenID Connect (OIDC) credentials. Update
both registries' publisher settings after a repository transfer or a workflow
or environment rename.

Keep two-factor authentication and account recovery available. The repository
API cannot verify these account settings.

## Temporary security exceptions

The ignored advisories and review deadline are in
[`.cargo/audit.toml`](../.cargo/audit.toml):

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

Review exceptions when Polars dependencies change and before the audit deadline.
Remove resolved advisories from the active exception list.
