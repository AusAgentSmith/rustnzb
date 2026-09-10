# Releasing rustnzb

GitHub is the release authority for rustnzb. Release tags use the `vX.Y.Z`
format and must point to a commit reachable from `main`.

## Release checklist

1. Update the workspace version in `Cargo.toml` and any user-visible version
   metadata.
2. Run the checks in [DEVELOPMENT.md](DEVELOPMENT.md).
3. Merge the release change to `main` through a reviewed pull request.
4. Create and push an annotated `vX.Y.Z` tag on that `main` commit.
5. Monitor the GitHub Actions release workflow.
6. Verify the GitHub release assets, checksums, and multi-architecture GHCR
   image:

   ```text
   ghcr.io/thedancingdeveloper-org/rustnzbd:vX.Y.Z
   ghcr.io/thedancingdeveloper-org/rustnzbd:latest
   ```

GitHub generates release notes from the tagged history. Review them before
publishing; do not add version-specific release-note files to the repository.

## Standalone nzb-* crates

The seven `nzb-*` crates under `crates/` are published to crates.io from
standalone repositories (one per crate, named in `ci/nzb-crates.txt`, URL in
each crate's `repository` field). This monorepo is the head:

- **The monorepo owns version numbers.** Change a crate's `version` only in
  `crates/<name>/Cargo.toml`; the standalone repos never originate a bump.
  Keep each new version strictly above the crates.io maximum for that crate.
- **Standalone repos are sync targets.** `ci/tasks/sync-crate <name>` exports
  `crates/<name>` at `HEAD` and opens a pull request on that crate's standalone
  repo. It never publishes and never pushes to the monorepo.
- **Publish only from a synced tag.** After a sync PR merges, publish that
  crate from a `vX.Y.Z` tag on the standalone repo — never from an out-of-sync
  tree.
- **Drift is gated.** The scheduled `crate-drift` job
  (`.github/workflows/quality-schedule.yml`, via `ci/tasks/crate-drift`) fails
  when any standalone repo's `src/` or package version differs from the
  monorepo. A failure means a crate changed here without a sync; run
  `ci/tasks/sync-crate <name>`, merge, and publish from the tag.

## Rollback

Do not move or replace a published tag. Revert the faulty change on `main`,
release a new patch version, and publish a new tag. Operators should pin an
immutable image tag rather than relying on `latest` for controlled rollouts.
