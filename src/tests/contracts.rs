//! Meta-contracts: the tests that keep the other tests honest.
//!
//! Standards citing tests that exist, sweeps that are documented, exemptions
//! that expire, inventories that catch a deletion (§6.10). Everything else
//! that used to live in this one file has its own module now: individual
//! constant values in `constants.rs`, frontend/backend agreement in
//! `frontend_parity.rs`, the deploy pipeline in `ci.rs`, the memory and
//! per-message-cost arithmetic in `memory_budget.rs`.

use super::*;

/// Every `.rs` file under `src/`, found recursively, skipping `src/tests/`.
///
/// A module split into a directory (`src/security/`, and others as they
/// follow) has to stay exactly as visible to these contracts as one that is
/// still a single file — a dead export or a missing header comment does not
/// stop being a problem because the module it lives in grew a `mod.rs`.
/// `src/tests/` is excluded the same way it always was when it was one
/// unwalked directory entry: this suite's own code is not the production
/// surface these contracts are about.
fn rust_source_files(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).expect("directory should be readable") {
        let path = entry.expect("a readable entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            files.extend(rust_source_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}

/// Every test named in the standards actually exists.
///
/// `ENGINEERING-STANDARDS.md` opens by asserting "every rule here is enforced
/// by a test in `src/tests.rs`", and its Enforcing-tests table names them one
/// by one. A rule whose named enforcer has been renamed or deleted is an
/// unenforced rule wearing an enforced rule's clothes — and the table reads
/// exactly the same either way.
#[test]
fn every_test_the_standards_name_exists() {
    const STANDARDS: &str = include_str!("../../ENGINEERING-STANDARDS.md");
    const CLAUDE_MD: &str = include_str!("../../CLAUDE.md");

    let mut missing: Vec<String> = Vec::new();

    for doc in [STANDARDS, CLAUDE_MD] {
        // Test names are cited in backticks, and are snake_case identifiers
        // long enough not to collide with prose or field names.
        for cited in doc.split('`').skip(1).step_by(2) {
            let looks_like_a_test = cited.len() > 12
                && cited.contains('_')
                && !cited.contains(' ')
                && !cited.contains("::")
                && !cited.contains('(')
                && !cited.contains('.')
                && cited
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');

            // Only names that read like assertions, not constants (which are
            // SCREAMING_CASE and already excluded) or field names.
            if !looks_like_a_test {
                continue;
            }

            let defined = suite_source().contains(&format!("fn {cited}("));
            let is_a_test_name = cited.starts_with("test_")
                || cited.contains("_must_")
                || cited.contains("_is_")
                || cited.contains("_are_")
                || cited.contains("_never_")
                || cited.contains("_cannot_")
                || cited.contains("_does_not_")
                || cited.contains("_matches_")
                || cited.contains("_releases_")
                || cited.contains("_exists_")
                || cited.contains("_still_")
                || cited.contains("_keeps_")
                || cited.contains("_holds_")
                || cited.contains("_fade")
                || cited.contains("_roster_");

            if is_a_test_name && !defined && !missing.contains(&cited.to_string()) {
                missing.push(cited.to_string());
            }
        }
    }

    assert!(
        missing.is_empty(),
        "the docs name these tests, but nothing defines them — either the test \
         was renamed and the doc not updated, or the rule is unenforced: {missing:?}"
    );
}

/// Every watched-levers row carries a reopen condition.
///
/// §9.4's table is the mechanism that keeps a parked decision from fossilising
/// into lore. A row with no reopen condition is exactly the "decided, then
/// forgotten" failure the registry exists to prevent, so the table's *shape* is
/// checked rather than trusted.
#[test]
fn every_parked_decision_records_how_to_reopen_it() {
    const STANDARDS: &str = include_str!("../../ENGINEERING-STANDARDS.md");

    let table = STANDARDS
        .split_once("| Lever | Status | Reopen when |")
        .map(|(_, rest)| rest)
        .expect("§9.4 should contain the watched-levers table");

    let mut incomplete: Vec<String> = Vec::new();
    let mut rows = 0;

    for line in table.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            if rows > 0 {
                break; // end of the table
            }
            continue;
        }
        // The header separator.
        if line.starts_with("| ---") {
            continue;
        }

        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 3 {
            continue;
        }
        rows += 1;

        let (lever, status, reopen) = (cells[0], cells[1], cells[2]);
        if reopen.len() < 20 || status.len() < 10 || lever.is_empty() {
            incomplete.push(lever.to_string());
        }
    }

    assert!(
        rows >= 8,
        "the registry should not have shrunk; found {rows} rows"
    );
    assert!(
        incomplete.is_empty(),
        "these watched-levers rows lack a real status or reopen condition, which \
         is how a parked decision becomes lore: {incomplete:?}"
    );
}

/// No test in this suite is a dummy: one that cannot fail, or one that cannot
/// even try.
///
/// Two shapes, both worth catching mechanically rather than by review. A
/// **tautology** — `is_ok() || is_err()`, `x == x`, `assert!(true)` — passes
/// whatever the code does, which reports coverage it does not provide (§6.4).
/// An **empty body** is the same failure with the assertion missing
/// altogether: `test_message_rate_limit_constant() {}` compiled, ran, and
/// passed on every commit since it was written, asserting nothing about a
/// rate limit or anything else. Neither is hypothetical — both were found
/// live in this file.
#[test]
fn no_assertion_in_this_suite_is_a_tautology() {
    // Each form is stored in halves and joined at run time, so the file never
    // literally contains the pattern it forbids. Written whole, this sweep
    // failed on its own definition — §6.7, for the third time in this suite.
    const TAUTOLOGY_HALVES: &[(&str, &str)] = &[
        ("is_ok() ", "|| result.is_err()"),
        ("is_err() ", "|| result.is_ok()"),
        ("assert!(", "true)"),
        ("assert_eq!(", "true, true)"),
        ("assert!(1 ", "== 1)"),
    ];

    let needles: Vec<String> = TAUTOLOGY_HALVES
        .iter()
        .map(|(head, tail)| format!("{head}{tail}"))
        .collect();

    let mut found: Vec<String> = Vec::new();
    let suite = suite_source();
    let lines: Vec<&str> = suite.lines().collect();

    for (number, line) in lines.iter().enumerate() {
        let code = line.split("//").next().unwrap_or(line);
        if !code.contains("assert") {
            continue;
        }
        for needle in &needles {
            if code.contains(needle.as_str()) {
                found.push(format!("line {}: {}", number + 1, code.trim()));
            }
        }
    }

    // An empty body: a `fn name(...) {` immediately followed by a line whose
    // only content is `}`. A real test's opening line is never its closing
    // one — this only matches a body with literally nothing between them.
    for (number, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let opens_a_test_fn = (trimmed.starts_with("fn ") || trimmed.starts_with("async fn "))
            && trimmed.contains('(')
            && trimmed.ends_with("{}");
        if opens_a_test_fn {
            let is_test = number > 0 && lines[number - 1].trim().starts_with('#');
            if is_test {
                found.push(format!("line {}: {}", number + 1, trimmed));
            }
        }
    }

    assert!(
        found.is_empty(),
        "these assertions cannot fail, or these test bodies cannot even try, \
         so they test nothing: {found:#?}"
    );
}

/// Every crate declared in `Cargo.toml` is actually used.
///
/// A dependency nobody imports is install time, build time and supply-chain
/// surface for nothing — and the freshness and audit sweeps have to keep
/// tracking it. Four such crates were deleted from this project once already
/// (`metrics`, `metrics-exporter-prometheus`, `async-trait`, `hyper`); this is
/// what stops the fifth.
#[test]
fn every_declared_dependency_is_used() {
    const CARGO_TOML: &str = include_str!("../../Cargo.toml");

    // Crates a build consumes without an `use` of its own.
    const KNOWN_INDIRECT: &[(&str, &str)] = &[
        ("axum-server", "used as `axum_server::Server` in main.rs"),
        (
            "tower",
            "test-only: `tower::util::ServiceExt` for `oneshot`",
        ),
        (
            "url",
            "test-only: parsing WebSocket URLs in the integration tests",
        ),
    ];

    // Read from disk rather than a hand-written list of `include_str!`s: that
    // list went stale the moment `startup.rs` was split out of `main.rs`, and a
    // sweep that silently stops seeing a module reports the crates it uses as
    // dead.
    // Walked, not listed: the suite became a directory of modules, and a
    // non-recursive read stopped seeing them — which reported every crate only
    // the tests use as dead.
    fn read_rust_files(dir: &std::path::Path, into: &mut String) {
        for entry in std::fs::read_dir(dir).expect("a readable directory") {
            let path = entry.expect("a readable entry").path();
            if path.is_dir() {
                read_rust_files(&path, into);
            } else if path.extension().is_some_and(|e| e == "rs") {
                into.push_str(&std::fs::read_to_string(&path).expect("a readable module"));
                into.push('\n');
            }
        }
    }

    let mut sources = String::new();
    read_rust_files(
        std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src")),
        &mut sources,
    );

    let mut unused: Vec<String> = Vec::new();

    for line in CARGO_TOML.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with('[') || !line.contains('=') {
            continue;
        }
        let Some((name, _)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        // Only dependency lines: keys of the package table are not crates.
        if !line.contains('"') && !line.contains('{') {
            continue;
        }
        if [
            "name",
            "version",
            "edition",
            "rust-version",
            "description",
            "license",
            "publish",
            "lto",
            "codegen-units",
            "strip",
            "panic",
        ]
        .contains(&name)
        {
            continue;
        }

        let ident = name.replace('-', "_");
        let referenced = sources.contains(&format!("{ident}::"))
            || sources.contains(&format!("use {ident}"))
            || sources.contains(&format!("extern crate {ident}"));

        if !referenced && !KNOWN_INDIRECT.iter().any(|(k, _)| *k == name) {
            unused.push(name.to_string());
        }
    }

    assert!(
        unused.is_empty(),
        "these crates are declared but never referenced — delete them or record \
         why they are needed indirectly: {unused:?}"
    );

    // Self-cleaning, the way the reference suite's pinned-with-reason list is:
    // an exemption must not outlive its reason.
    for (name, reason) in KNOWN_INDIRECT {
        assert!(
            CARGO_TOML.contains(name),
            "`{name}` is exempted as an indirect dependency ({reason}) but is no \
             longer in Cargo.toml — delete the exemption"
        );
    }
}

/// Every contract test is documented somewhere a future session will look.
///
/// The other half of the phantom-enforcement problem: a sweep can exist and
/// guard something real while nothing says what or why, so the reasoning lives
/// only in the file and is one refactor from being lore. Ported from the
/// standards-enforcement contract on `cameronaaron.com`.
#[test]
fn every_contract_test_is_documented() {
    const STANDARDS: &str = include_str!("../../ENGINEERING-STANDARDS.md");
    const CLAUDE_MD: &str = include_str!("../../CLAUDE.md");
    let docs = format!("{STANDARDS}\n{CLAUDE_MD}");

    // The sweeps: tests whose names read as a rule about the whole codebase
    // rather than an example of one behaviour.
    const MARKERS: &[&str] = &[
        "every_",
        "no_",
        "the_client_",
        "the_page_",
        "the_workflows_",
        "client_",
    ];

    let mut undocumented: Vec<&str> = Vec::new();

    let suite = suite_source();
    for line in suite.lines() {
        let line = line.trim();
        let Some(rest) = line
            .strip_prefix("async fn ")
            .or_else(|| line.strip_prefix("fn "))
        else {
            continue;
        };
        let Some((name, _)) = rest.split_once('(') else {
            continue;
        };
        if !MARKERS.iter().any(|m| name.starts_with(m)) {
            continue;
        }
        // Helpers are not contracts.
        if name.ends_with("_source") || name.contains("without_comments") {
            continue;
        }
        if !docs.contains(name) {
            undocumented.push(name);
        }
    }

    assert!(
        undocumented.is_empty(),
        "these sweeps guard something but nothing documents what or why; add \
         them to the Enforcing-tests table: {undocumented:?}"
    );
}

/// The coverage exemptions are justified, current, and honest about their size.
///
/// Ported from the pinned-dependency list on `cameronaaron.com`, which fails
/// when a pin catches up to latest so an exemption can never outlive its
/// reason. Three ways this registry can rot, each checked:
///
///   1. An entry with no reason — a number being hidden rather than explained.
///   2. An entry naming a path that no longer exists.
///   3. A third entry appearing. Only `tests.rs` and `main.rs` are exempt, and
///      both for the same structural reason — one is the suite, the other is an
///      entry point a test can never call. Everything else reached 100%, so a
///      new entry is a claim that something is untestable, and the first
///      question is whether the code can move instead (§6.1c).
#[test]
fn coverage_exemptions_are_justified_and_current() {
    const EXEMPTIONS: &str = include_str!("../../scripts/coverage-exemptions.toml");
    const COVERAGE_SH: &str = include_str!("../../scripts/coverage.sh");

    let mut entries = 0;
    for block in EXEMPTIONS.split("[[exempt]]").skip(1) {
        entries += 1;

        let path = block
            .split_once("path = \"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .map(|(p, _)| p)
            .expect("every exemption names a path");

        assert!(
            std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR")))
                .join(path)
                .exists(),
            "{path} is exempted from coverage but no longer exists — an \
             exemption must not outlive the thing it exempts"
        );

        let reason = block
            .split_once("reason = \"\"\"")
            .and_then(|(_, rest)| rest.split_once("\"\"\""))
            .map(|(r, _)| r.trim())
            .unwrap_or_default();

        assert!(
            reason.len() > 80,
            "{path} is exempted without a real reason; an exclusion with no \
             explanation is a number being hidden"
        );
    }

    assert_eq!(
        entries, 2,
        "only the two whole-file exemptions remain. `session.rs` had 21 line \
         exemptions, then 12, then 3, then none — every one turned out to be a \
         misplaced line rather than an untestable one. If a third file appears \
         here, the question to ask first is whether the code can move (§6.1c)."
    );

    // Whole-file exclusions must be exactly the ones the script passes to
    // tarpaulin, or the registry describes a gate that is not running.
    for path in ["src/tests", "src/main.rs"] {
        assert!(
            COVERAGE_SH.contains(path),
            "{path} is exempted in the registry but not excluded by coverage.sh"
        );
        assert!(
            EXEMPTIONS.contains(path),
            "coverage.sh excludes {path} but the registry does not explain why"
        );
    }
}

/// Every `§` and `constraint #` pointer resolves to something that exists.
///
/// The docs and the source comments are dense with them — `§5.11`,
/// `constraint #12`, `§1.4a` — and several rules lean on another by name. A
/// section renumbered or a constraint deleted turns every pointer to it into a
/// lie, silently, with a green gate: nothing else reads them.
///
/// Ported from the docs-cross-reference contract on `cameronaaron.com`. The
/// addition here is that Rust source comments are checked too, because in this
/// codebase the citation usually lives next to the code it justifies rather
/// than in the document.
#[test]
fn every_section_reference_resolves() {
    const STANDARDS: &str = include_str!("../../ENGINEERING-STANDARDS.md");
    const CLAUDE_MD: &str = include_str!("../../CLAUDE.md");

    // Sections that exist: `## 5. …` and `### 5.11 …`.
    let mut sections: std::collections::HashSet<String> = std::collections::HashSet::new();
    for line in STANDARDS.lines() {
        let trimmed = line.trim_start_matches('#').trim_start();
        if !line.starts_with('#') {
            continue;
        }
        if let Some((number, _)) = trimmed.split_once(' ')
            && number.chars().next().is_some_and(|c| c.is_ascii_digit())
        {
            sections.insert(number.trim_end_matches('.').to_string());
        }
    }
    assert!(
        sections.len() > 40,
        "the standards should have many numbered sections; found {}",
        sections.len()
    );

    // Constraints that exist: `### 12. …` in CLAUDE.md.
    let mut constraints: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for line in CLAUDE_MD.lines() {
        if let Some(rest) = line.strip_prefix("### ")
            && let Some((number, _)) = rest.split_once('.')
            && let Ok(n) = number.parse::<u32>()
        {
            constraints.insert(n);
        }
    }
    assert!(
        constraints.len() > 20,
        "CLAUDE.md should list many constraints; found {}",
        constraints.len()
    );

    // Every corpus that cites them: both docs, and every Rust module.
    let mut corpus = vec![
        (
            "ENGINEERING-STANDARDS.md".to_string(),
            STANDARDS.to_string(),
        ),
        ("CLAUDE.md".to_string(), CLAUDE_MD.to_string()),
    ];
    let source_dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    for path in rust_source_files(source_dir) {
        let name = path
            .strip_prefix(source_dir)
            .unwrap()
            .to_string_lossy()
            .to_string();
        corpus.push((name, std::fs::read_to_string(&path).expect("readable")));
    }

    let mut broken: Vec<String> = Vec::new();

    for (name, body) in &corpus {
        for (index, _) in body.match_indices('§') {
            let rest = &body[index + '§'.len_utf8()..];
            let reference: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.' || c.is_ascii_lowercase())
                .collect();
            let reference = reference.trim_end_matches('.').to_string();
            if reference.is_empty() {
                continue;
            }
            if !sections.contains(&reference) {
                broken.push(format!("{name}: §{reference}"));
            }
        }

        for (index, _) in body.match_indices("constraint #") {
            let rest = &body[index + "constraint #".len()..];
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let Ok(number) = digits.parse::<u32>() else {
                continue;
            };
            if !constraints.contains(&number) {
                broken.push(format!("{name}: constraint #{number}"));
            }
        }
    }

    broken.sort();
    broken.dedup();
    assert!(
        broken.is_empty(),
        "these pointers name a section or constraint that does not exist — a \
         renumber or a deletion left them behind: {broken:#?}"
    );
}

/// §8 — every module says what it is for, and none is named for nothing.
///
/// A module called `utils` is a module whose contents nobody decided on. The
/// header comment is the other half: a file whose job is not stated in it is a
/// file whose job drifts.
#[test]
fn every_module_is_named_for_its_job_and_says_what_it_is() {
    const FORBIDDEN: &[&str] = &[
        "utils", "helpers", "common", "misc", "shared", "core", "lib2",
    ];

    let source_dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let mut checked = 0;

    for path in rust_source_files(source_dir) {
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        checked += 1;

        assert!(
            !FORBIDDEN.contains(&name.as_str()),
            "`{name}.rs` is named for nothing — a module called that is one \
             whose contents nobody decided on (§8)"
        );
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "`{name}.rs` should be snake_case"
        );

        // `tests.rs` is the suite; the rest must state their job at the top.
        if name == "tests" {
            continue;
        }
        let body = std::fs::read_to_string(&path).expect("a readable module");
        let header: String = body.lines().take_while(|l| l.starts_with("//!")).collect();
        assert!(
            header.len() > 60,
            "`{name}.rs` has no header comment saying what it owns (§8.1)"
        );
    }

    assert!(checked >= 15, "expected every module to be checked");
}

/// Nothing is exported that only its own test uses.
///
/// The failure mode: a function superseded months ago, still compiling, still
/// covered — by the test written for it. A 100% coverage gate cannot tell that
/// apart from live code, which is exactly why it needs its own sweep.
///
/// Ported from the dead-logic-export contract on `cameronaaron.com`.
#[test]
fn nothing_is_public_only_for_its_own_test() {
    let source_dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));

    let mut production = String::new();
    let mut declarations: Vec<(String, String)> = Vec::new();

    for path in rust_source_files(source_dir) {
        let name = path
            .strip_prefix(source_dir)
            .unwrap()
            .to_string_lossy()
            .to_string();
        let body = std::fs::read_to_string(&path).expect("a readable module");

        production.push_str(&body);
        production.push('\n');

        for line in body.lines() {
            let line = line.trim();
            let Some(rest) = line
                .strip_prefix("pub fn ")
                .or_else(|| line.strip_prefix("pub async fn "))
                .or_else(|| line.strip_prefix("pub(crate) fn "))
                .or_else(|| line.strip_prefix("pub(crate) async fn "))
            else {
                continue;
            };
            // Strip generics: `forward_broadcasts<S: FrameSink>` is declared
            // with them and referred to without.
            let fn_name = rest
                .split_once('(')
                .map(|(head, _)| head)
                .unwrap_or(rest)
                .split('<')
                .next()
                .unwrap_or_default()
                .trim();
            if !fn_name.is_empty() {
                declarations.push((name.clone(), fn_name.to_string()));
            }
        }
    }

    assert!(
        declarations.len() > 30,
        "expected to find many exported functions; found {}",
        declarations.len()
    );

    // Functions that exist only for the suite, by construction.
    const TEST_ONLY: &[(&str, &str)] = &[("startup.rs", "generate_random_room_name")];

    let mut dead: Vec<String> = Vec::new();
    for (module, function) in &declarations {
        if TEST_ONLY.iter().any(|(m, f)| m == module && f == function) {
            continue;
        }

        // Count *references*, not calls: a handler is mounted as
        // `get(ws_handler)` and a validator is passed as
        // `and_then(sanitize_reply)` — neither is followed by a paren.
        let referenced = production
            .match_indices(function.as_str())
            .filter(|(index, _)| {
                let before = production[..*index].chars().next_back();
                let after = production[index + function.len()..].chars().next();
                let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric() && c != '_');
                boundary(before) && boundary(after)
            })
            .count();

        // The declaration itself is one of those references.
        if referenced <= 1 {
            dead.push(format!("{module}::{function}"));
        }
    }

    // Self-cleaning: an exemption must not outlive the thing it exempts.
    for (module, function) in TEST_ONLY {
        assert!(
            declarations
                .iter()
                .any(|(m, f)| m == module && f == function),
            "{module}::{function} is exempted as test-only but no longer exists"
        );
    }

    assert!(
        dead.is_empty(),
        "these are exported but called only from tests — a function whose only \
         caller is the test written for it is dead code with a green coverage \
         report: {dead:#?}"
    );
}
