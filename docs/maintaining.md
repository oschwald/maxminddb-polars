# Maintainer operations

This file records repository settings that cannot be enforced by committed
files. Re-audit every item before the first public release and after ownership
or publishing changes.

## Bootstrap record

- Repository owner: `oschwald`
- Repository name: `maxminddb-polars`
- Visibility: public
- Default branch: `main`
- Settings last verified: 2026-08-24
- License: ISC
- Rust release toolchain: 1.96.0
- Python floor: 3.10
- Python Polars range: 1.43.x
- Rust Polars/pyo3-polars: 0.54.4/0.27.0
- maxminddb: 0.30.1
- MaxMind-DB fixture commit: `e1120013c4b5cbc830b958b2b7e73fba444d316d`

Exact-name checks on 2026-08-23 found no `maxminddb-polars` project on PyPI,
crates.io, npm, or `oschwald` GitHub. These observations do not reserve names.

## GitHub checklist

- [x] `main` is the default branch and Issues are enabled.
- [x] Dependency graph, Dependabot alerts, and security updates are enabled.
- [x] Private vulnerability reporting is enabled.
- [x] Secret scanning and push protection are enabled.
- [x] Code scanning is enabled and the CodeQL workflow is green.
- [x] The default Actions token permission is read-only.
- [x] Workflows cannot approve pull requests.
- [x] `main` blocks force pushes and deletion and requires pull requests.
- [x] Stable lint, Rust test, and Python smoke checks are required.
- [x] Relevant package changes run the complete distribution-validation workflow.
- [x] A `v*` tag ruleset blocks tag updates and deletion.
- [x] Workflow artifacts are retained for 30 days.
- [x] Merge, squash, and rebase merges are allowed; merged branches are deleted.

The `main` protection rule requires one approving review, resolved
conversations, an up-to-date branch, and these status checks:

- `Clippy`
- `Metadata and locks`
- `Prettier`
- `Rustfmt`
- `Rust unit tests`
- `Python 3.10 on ubuntu-latest`

`Validate distributions` is intentionally not required because the artifact
workflow is path-filtered; requiring it would block documentation-only pull
requests for a check that does not run.

Repository administrators retain a recovery bypass. Force pushes and branch
deletion remain disabled for everyone.

## Publishing checklist

- [x] A `pypi` environment accepts only `v*` tags and has no upload token.
- [x] A `release` environment accepts only `v*` tags and has no upload token.
- [x] The PyPI Trusted Publisher uses owner `oschwald`.
- [x] Its repository is `maxminddb-polars`.
- [x] Its workflow is `release.yml`.
- [x] Its environment is `pypi`.
- [x] Its PyPI project name is `maxminddb-polars`.
- [x] The crates.io trusted publisher uses owner `oschwald`, repository
      `maxminddb-polars`, workflow `release.yml`, and environment `release`.
- [ ] Maintainer accounts have two-factor authentication and recovery access.
- [x] A no-publish artifact and release rehearsal has passed.

The `0.1.0` release allocated the `maxminddb-polars` project name on both PyPI
and crates.io. Both registries publish subsequent releases from `release.yml`
with short-lived OIDC credentials.

## Temporary security exceptions

`cargo audit` temporarily ignores these transitive advisories in
`.cargo/audit.toml`:

- `RUSTSEC-2025-0141`: unmaintained `bincode` 2.0.1 is a transitive dependency
  of Rust Polars 0.55 and has no patched release.
- `RUSTSEC-2026-0194` and `RUSTSEC-2026-0195`: `quick-xml` is locked only
  through the disabled cloud features of Polars' `object_store`; it is absent
  from every built target (`cargo tree --target all -i quick-xml`) and the
  plugin does not parse XML.

Re-evaluate these three exception IDs with the next coordinated Polars update,
or by 2026-09-30, whichever comes first. The `0.1.2` update to PyO3 0.29.2
resolved `RUSTSEC-2026-0176` and `RUSTSEC-2026-0177`; those exceptions and the
corresponding Dependabot alerts are closed.
