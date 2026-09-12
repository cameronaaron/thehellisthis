# Release readiness — 2026-09-12

The identified build, dependency-audit and deployment-gate defects are fixed.
These changes have not been pushed or deployed to Cloudflare.

## Changes

- One Docker recipe serves both entry points, includes the workspace and assets,
  builds from Cargo.lock, and runs as non-root.
- Frozen Worker installs use working pnpm 11.22.0.
- Updated h2 to 0.4.16, sharp to 0.35.4 and yanked chacha20 to 0.10.2.
  Regenerated the committed Nova WASM artifact from the updated lockfile.
- Forward admin and metrics secrets into the container; keep unavailable
  responses uncached. Four Worker tests exercise the actual handler/constructor
  with Cloudflare transport stubbed.
- CI tests the entire workspace, Worker behavior and production container.
  Manual deployment runs release, coverage, audit and container checks before
  publishing, without pruning unrelated Docker caches.
- Fixed portable temporary-file creation and unsafe cleanup on smoke-build
  failure. Regression tests cover Docker entry-point drift, missing deployment
  checks and cleanup before a server has started.
- Enabled and verified GitHub protection for main: an up-to-date PR must pass
  all five CI jobs, including for administrators; force pushes/deletion disabled.
  No additional reviewer approval is required for this single-maintainer flow.

## Verification

- Formatting and workspace Clippy with warnings denied: passed.
- Workspace tests: 816 passed; the two excluded tests passed separately.
- Release build and release-only RLN proof/slashing test: passed.
- Exact coverage script: 85.01%, 2178/2562 lines; existing 84% floor unchanged.
- Worker frozen install, typecheck and four regression tests: passed.
- Rust audits of both lockfiles and Worker high-severity audit: clean.
- Full Git history secret scan: 194 commits, no leaks found before this commit.
- Native smoke: four arrivals/departures, zero retained connection slots.
- Container HTTP/assets/authentication/non-root/shutdown: passed on Linux ARM64.
- Synthetic container WebSocket conversation passed at 256 MiB and 1/16 CPU;
  this is a smoke test, not a capacity benchmark.
- Deployment-target Linux AMD64 container build and all runtime checks: passed.

## Remaining operational boundary

Live Cloudflare behavior and configured secret values were not tested or changed.
Provision the desired admin/metrics/operator secrets as described in README.
Merge these changes through the protected PR flow, then verify the deployment.
No maximum-load claim or independent cryptographic security audit is implied.
Known coverage gaps remain documented in scripts/coverage.sh.

## Advisory evidence

- [pnpm 11.12.0 packaging failure](https://github.com/pnpm/pnpm/issues/12955)
- [h2 advisory](https://rustsec.org/advisories/RUSTSEC-2026-0258)
- [sharp/libheif advisory](https://github.com/advisories/GHSA-rgj7-g3m4-5g8c)
