//! The deploy pipeline: what CI is and is not trusted to do, and whether its
//! toolchain can actually run the thing it is gating.

use super::*;

/// CI does not deploy, and must not start again.
///
/// Deploying moved to Cloudflare's own Git integration. What it replaced kept
/// producing the same failure shape: the Rust gate green, and the deploy step
/// failing afterwards on something nothing else looked at — a Node version,
/// then a token permission. Both took an afternoon to find because every signal
/// a person reads said the commit was fine.
///
/// The credential is what makes this worth pinning rather than just deleting.
/// A workflow holding a deploy token is the most valuable thing in the
/// repository to an attacker who lands a pull request, and "we removed it" is
/// only true until somebody adds it back for a good reason.
#[test]
fn ci_holds_no_deploy_credential_and_does_not_deploy() {
    const CI: &str = include_str!("../../.github/workflows/ci.yml");

    let workflows = std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/.github/workflows"))
        .expect("the workflows directory should exist");

    let mut files = Vec::new();
    for entry in workflows {
        let path = entry.expect("a readable directory entry").path();
        let body = std::fs::read_to_string(&path).expect("a readable workflow");
        files.push((
            path.file_name().unwrap().to_string_lossy().to_string(),
            body,
        ));
    }

    for (name, body) in &files {
        let commands = strip_hash_comments(body);
        assert!(
            !commands.contains("wrangler"),
            "{name} invokes wrangler; deploying is Cloudflare's Git integration \
             now, and a workflow that deploys needs a token"
        );
        assert!(
            !body.contains("CLOUDFLARE_API_TOKEN") && !body.contains("CLOUDFLARE_ACCOUNT_ID"),
            "{name} references a Cloudflare credential — CI has no reason to \
             hold one, and a workflow secret is reachable from any pull request \
             that can change a workflow"
        );
    }

    // The gate still has to run on the push that Cloudflare deploys from, or
    // nothing checks the commit that actually ships.
    assert!(
        CI.contains("push:") && CI.contains("branches: [main]"),
        "the gate must run on pushes to main, since that is what deploys"
    );
}

/// The Node that CI installs is one the toolchain can actually run on.
///
/// This is the test that would have saved the afternoon. `wrangler` requires
/// Node >= 22 and pnpm 11 needs `node:sqlite`, which arrived in 22. Both
/// workflows pinned Node 20. The Rust gate — fmt, clippy, 600 tests, release
/// build, coverage — went green on every push, and then the *deploy step*
/// failed, so nothing reached the live site while every signal a person looks
/// at said the commit was fine.
///
/// §6.1 says the gate exists to answer "will this deploy". A gate that cannot
/// see the deploy's own requirements is not answering it. The requirement lives
/// in `cloudflare/package.json` under `engines`, once, and this asserts the
/// workflow agrees with it. Deploying moved to Cloudflare's Git integration,
/// but the Worker typecheck still runs here and still needs a Node that pnpm
/// and wrangler can run on.
#[test]
fn ci_node_version_satisfies_the_toolchain() {
    const PACKAGE_JSON: &str = include_str!("../../cloudflare/package.json");
    const CI: &str = include_str!("../../.github/workflows/ci.yml");

    // The declared floor, e.g. `"node": ">=22"`.
    let required: u32 = PACKAGE_JSON
        .split_once("\"node\":")
        .and_then(|(_, rest)| rest.split_once('"'))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(spec, _)| {
            spec.trim_start_matches(['>', '=', '^', '~', ' '])
                .to_string()
        })
        .and_then(|spec| spec.split('.').next().unwrap_or_default().parse().ok())
        .expect("cloudflare/package.json should declare engines.node");

    assert!(
        required >= 22,
        "wrangler needs Node 22 or newer; the declared floor is {required}"
    );

    for (name, workflow) in [("ci.yml", CI)] {
        let mut found = 0;
        for (index, _) in workflow.match_indices("node-version: '") {
            let rest = &workflow[index + "node-version: '".len()..];
            let Some(end) = rest.find('\'') else { continue };
            let major: u32 = rest[..end]
                .split('.')
                .next()
                .unwrap_or_default()
                .parse()
                .unwrap_or_else(|_| panic!("{name} has an unparseable node-version"));

            assert!(
                major >= required,
                "{name} installs Node {major}, below the {required} the Worker \
                 toolchain requires — the Rust gate would still pass and the \
                 deploy would still fail"
            );
            found += 1;
        }
        assert!(found > 0, "{name} should pin a Node version");
    }
}

/// The deploy's install command is the one the lockfile format belongs to.
///
/// Switching package managers is easy to do halfway: a `pnpm-lock.yaml` in the
/// tree and an `npm ci` in the workflow installs from `package.json` alone,
/// silently resolving different versions than anything anyone tested.
#[test]
fn the_workflows_install_with_the_lockfile_that_exists() {
    const CI: &str = include_str!("../../.github/workflows/ci.yml");
    const DEPLOY_SH: &str = include_str!("../../deploy.sh");

    for (name, script) in [("ci.yml", CI), ("deploy.sh", DEPLOY_SH)] {
        // Tokens, and only from the commands — not substrings, and not prose.
        // This test failed twice before it passed once: `pnpm install` contains
        // "npm install", and then a comment mentioning "RUSTSEC and npm
        // advisories" matched the token. §6.7, twice, in the same afternoon.
        let commands = strip_hash_comments(script);
        let invoked: Vec<&str> = commands
            .split_whitespace()
            .filter(|word| *word == "npm" || *word == "npx")
            .collect();

        assert!(
            invoked.is_empty(),
            "{name} invokes {invoked:?}, but the repository's lockfile is \
             pnpm-lock.yaml — npm installs from package.json alone and npx \
             resolves outside the pnpm store, either way running versions \
             nothing was tested against"
        );
    }

    assert!(
        CI.contains("--frozen-lockfile"),
        "CI must install from the lockfile, not update it"
    );
}

/// `@types/node` describes the Node the workflows actually install.
///
/// Types are a claim about the runtime. A `@types/node` ahead of the installed
/// Node promises APIs that will not be there, and one behind hides APIs that
/// are — either way the typecheck is answering a question about a different
/// machine than the one the build runs on.
#[test]
fn the_node_types_match_the_node_ci_installs() {
    const PACKAGE_JSON: &str = include_str!("../../cloudflare/package.json");
    const CI: &str = include_str!("../../.github/workflows/ci.yml");

    let types_major: u32 = PACKAGE_JSON
        .split_once("\"@types/node\":")
        .and_then(|(_, rest)| rest.split_once('"'))
        .and_then(|(_, rest)| rest.split_once('"'))
        .map(|(spec, _)| {
            spec.trim_start_matches(['^', '~', '>', '=', ' '])
                .to_string()
        })
        .and_then(|spec| spec.split('.').next().unwrap_or_default().parse().ok())
        .expect("cloudflare/package.json should depend on @types/node");

    let ci = strip_hash_comments(CI);
    let installed: u32 = ci
        .split_once("node-version: '")
        .and_then(|(_, rest)| rest.split_once('\''))
        .and_then(|(version, _)| version.split('.').next().unwrap_or_default().parse().ok())
        .expect("ci.yml should pin a node-version");

    assert_eq!(
        types_major, installed,
        "@types/node is for Node {types_major} but CI installs Node {installed}; \
         the typecheck would be describing a runtime nobody runs"
    );
}

/// The container image's base images track latest on purpose, and stay that
/// way.
///
/// This project's stated policy is to run at latest rather than pin and
/// periodically catch up — Dependabot, `cargo update`, `cargo audit`/`pnpm
/// audit` on a weekly schedule. A `FROM` line naming a specific version
/// (`rust:1.97-bookworm`, `debian:bookworm-slim`) is that policy quietly
/// reversed: it stops moving the moment it is written, and a base image that
/// stops moving accumulates whatever it shipped with, forever, until someone
/// remembers to bump it by hand. `latest`/`slim`/`stable` move on every
/// rebuild instead.
///
/// Deliberately narrow: this checks the *tag*, not the CVE count a scanner
/// reports for it. A scanner's count is a snapshot of today's image and
/// changes on its own as upstream patches land — nothing in a test suite
/// should assert on a number that is true right now and false by the next
/// `docker pull`.
#[test]
fn these_images_track_latest_on_purpose() {
    const DOCKERFILE: &str = include_str!("../../Dockerfile.cloudflare");

    let from_lines: Vec<&str> = DOCKERFILE
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("FROM "))
        .collect();

    assert!(
        from_lines.len() >= 2,
        "expected a multi-stage build with at least a builder and a runtime \
         stage; found {}",
        from_lines.len()
    );

    // The moving tags this policy allows. Anything else is a pin.
    const MOVING_TAGS: &[&str] = &["latest", "slim", "stable", "stable-slim"];

    let mut pinned: Vec<&str> = Vec::new();
    for line in &from_lines {
        // `FROM rust:slim AS builder` -> `rust:slim`
        let image = line
            .strip_prefix("FROM ")
            .and_then(|rest| rest.split(" AS ").next())
            .unwrap_or(line)
            .trim();

        let tag = image.rsplit_once(':').map_or("latest", |(_, tag)| tag);

        if !MOVING_TAGS.contains(&tag) {
            pinned.push(line);
        }
    }

    assert!(
        pinned.is_empty(),
        "these FROM lines name a specific version instead of a moving tag, \
         reversing the project's run-at-latest policy: {pinned:?}"
    );
}
