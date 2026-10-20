//! Guard tests that keep the rule registry, the specs in `docs/rules`, the `Violation`
//! impls, and the fixture corpus in agreement. Each of these is maintained by hand in a
//! different place, so drift between them should fail CI instead of shipping.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::rule_metadata::{RULE_SPECS, RuleSpecStatus};
use crate::test::test_path;
use crate::{Category, FixAvailability, LinterSettings, Rule, code_to_rule, rule_metadata};

/// Rules whose fixture is expected to produce no diagnostics. Every entry is a known gap;
/// remove it once the rule fires on its fixture (the test fails if an entry goes stale).
const SILENT_FIXTURE_ALLOWLIST: &[(&str, &str)] = &[
    // C065-C068 and C077 are registered but their check functions are empty, so they can
    // never report. Whether to implement or drop them is an open product decision.
    ("C065", "rule body is empty; never reports"),
    ("C066", "rule body is empty; never reports"),
    ("C067", "rule body is empty; never reports"),
    ("C068", "rule body is empty; never reports"),
    ("C077", "rule body is empty; never reports"),
    // X077 is registered but its check function is empty, so it can never report.
    ("X077", "rule body is empty; never reports"),
    // These rules have real implementations, but their fixtures do not currently trigger
    // them (the committed snapshots are empty). The fixtures need a firing case.
    ("C109", "fixture snapshot is empty"),
    ("X044", "fixture snapshot is empty"),
    ("X047", "fixture snapshot is empty"),
    ("X051", "fixture snapshot is empty"),
];

/// Registered rules that have no `docs/rules/<CODE>.yaml` spec and no fixture file.
const UNSPECCED_RULE_ALLOWLIST: &[(&str, &str)] = &[
    // BackslashBeforeCommand is registered under its legacy code instead of a canonical
    // S-code, so it has no spec, no rule metadata, no website entry, and no fixture (its
    // coverage is the rule-local snippet tests). Giving it a canonical code is a product
    // decision because it changes the code users see and suppress.
    (
        "SH-173",
        "registered under a legacy code with no spec or fixture",
    ),
];

fn category_dir(category: Category) -> &'static str {
    match category {
        Category::Correctness => "correctness",
        Category::Style => "style",
        Category::Performance => "performance",
        Category::Portability => "portability",
        Category::Security => "security",
    }
}

fn rule_variant_name(rule: Rule) -> String {
    format!("{rule:?}")
}

fn collect_rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("failed to read entry in {}: {error}", dir.display()))
            .path();
        if path.is_dir() {
            collect_rust_sources(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// Scans the crate sources for `impl Violation for ...` blocks and returns each rule's declared
/// `FIX_AVAILABILITY`, keyed by `Rule` variant name. A block without the constant uses the
/// trait default (`None`). There is no runtime registry of violation types, so this reads the
/// source the same way the website's rule table generator does.
fn declared_fix_availability() -> BTreeMap<String, Vec<(String, FixAvailability)>> {
    // Some violations live outside `src/rules`, e.g. parse-diagnostic rules.
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = Vec::new();
    collect_rust_sources(&src_root, &mut paths);
    paths.sort();

    let mut declared = BTreeMap::<String, Vec<(String, FixAvailability)>>::new();
    for path in paths {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        let mut rest = source.as_str();
        while let Some(start) = rest.find("impl Violation for ") {
            let block = &rest[start..];
            // Impl blocks are rustfmt-formatted, so the first column-zero `}` closes them.
            let end = block
                .find("\n}")
                .unwrap_or_else(|| panic!("unterminated Violation impl in {}", path.display()));
            let body = &block[..end];
            rest = &block[end..];

            let violation = body["impl Violation for ".len()..]
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .next()
                .unwrap_or_default()
                .to_owned();
            let rule = body
                .split("fn rule() -> Rule")
                .nth(1)
                .and_then(|after| after.split("Rule::").nth(1))
                .map(|after| {
                    after
                        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                        .next()
                        .unwrap_or_default()
                        .to_owned()
                })
                .unwrap_or_else(|| {
                    panic!(
                        "could not find `fn rule()` for {violation} in {}",
                        path.display()
                    )
                });
            let availability = if body.contains("FixAvailability::Always") {
                FixAvailability::Always
            } else if body.contains("FixAvailability::Sometimes") {
                FixAvailability::Sometimes
            } else {
                FixAvailability::None
            };
            declared
                .entry(rule)
                .or_default()
                .push((violation, availability));
        }
    }
    declared
}

fn fixture_path(rule: Rule) -> Option<PathBuf> {
    let dir = category_dir(rule.category());
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/test/fixtures");
    ["sh", "txt"]
        .into_iter()
        .map(|ext| Path::new(dir).join(format!("{}.{ext}", rule.code())))
        .find(|relative| fixtures_root.join(relative).is_file())
}

fn is_unspecced(rule: Rule) -> bool {
    UNSPECCED_RULE_ALLOWLIST
        .iter()
        .any(|(code, _)| *code == rule.code())
}

#[test]
fn every_rule_has_spec_metadata() {
    let mut problems = Vec::new();
    for rule in Rule::iter() {
        match (rule_metadata(rule).is_some(), is_unspecced(rule)) {
            (false, false) => problems.push(format!(
                "{} has no docs/rules/{}.yaml spec",
                rule.code(),
                rule.code()
            )),
            (true, true) => problems.push(format!(
                "{} now has a spec; remove it from UNSPECCED_RULE_ALLOWLIST",
                rule.code()
            )),
            _ => {}
        }
    }

    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn rule_spec_status_matches_registry() {
    let registered = Rule::iter().map(Rule::code).collect::<BTreeSet<_>>();
    let mut problems = Vec::new();

    for spec in RULE_SPECS {
        let is_registered = registered.contains(spec.code);
        let expects_registration = match spec.status {
            RuleSpecStatus::Implemented => true,
            RuleSpecStatus::Planned | RuleSpecStatus::Retired => false,
        };
        if expects_registration && !is_registered {
            problems.push(format!(
                "{} has no registered Rule; implement it or mark the spec `status: planned`",
                spec.code
            ));
        } else if !expects_registration && is_registered {
            problems.push(format!(
                "{} is registered but its spec says `status: {:?}`; drop the status field",
                spec.code, spec.status
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "docs/rules specs and the registry disagree:\n{}",
        problems.join("\n"),
    );
}

#[test]
fn spec_aliases_resolve_to_their_rule() {
    let mut problems = Vec::new();

    for spec in RULE_SPECS {
        if spec.status != RuleSpecStatus::Implemented {
            continue;
        }
        let expected = code_to_rule(spec.code).map(Rule::code);
        for alias in spec.legacy_code.iter().chain(spec.aliases) {
            let resolved = code_to_rule(alias).map(Rule::code);
            if resolved != expected {
                problems.push(format!(
                    "{alias} resolves to {resolved:?}, expected {expected:?}"
                ));
            }
        }
    }

    assert!(problems.is_empty(), "alias drift:\n{}", problems.join("\n"));
}

#[test]
fn fix_availability_matches_spec_fix_description() {
    let declared = declared_fix_availability();
    let mut problems = Vec::new();

    for rule in Rule::iter() {
        let name = rule_variant_name(rule);
        let impls = declared.get(&name).map(Vec::as_slice).unwrap_or_default();
        if impls.len() != 1 {
            problems.push(format!(
                "{} ({name}) should have exactly one `impl Violation`, found {impls:?}",
                rule.code()
            ));
            continue;
        }
        let has_fix = impls
            .iter()
            .any(|(_, availability)| *availability != FixAvailability::None);
        let documents_fix = rule_metadata(rule).is_some_and(|meta| meta.fix_description.is_some());

        if has_fix && !documents_fix {
            problems.push(format!(
                "{} declares FIX_AVAILABILITY but its spec has no fix_description",
                rule.code()
            ));
        }
        if documents_fix && !has_fix {
            problems.push(format!(
                "{} has a fix_description but every Violation declares FixAvailability::None",
                rule.code()
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "Violation::FIX_AVAILABILITY and docs/rules fix_description disagree:\n{}",
        problems.join("\n"),
    );
}

#[test]
fn rule_fixtures_produce_diagnostics_and_honor_fix_availability() {
    let declared = declared_fix_availability();
    let allowlist = SILENT_FIXTURE_ALLOWLIST
        .iter()
        .map(|(code, _)| *code)
        .collect::<BTreeSet<_>>();
    let mut problems = Vec::new();

    for rule in Rule::iter() {
        let Some(path) = fixture_path(rule) else {
            if is_unspecced(rule) {
                continue;
            }
            problems.push(format!(
                "{} has no fixture at resources/test/fixtures/{}/{}.sh",
                rule.code(),
                category_dir(rule.category()),
                rule.code()
            ));
            continue;
        };
        let (diagnostics, _) = test_path(&path, &LinterSettings::for_rule(rule))
            .unwrap_or_else(|error| panic!("failed to lint {}: {error}", path.display()));
        let diagnostics = diagnostics
            .into_iter()
            .filter(|diagnostic| diagnostic.rule == rule)
            .collect::<Vec<_>>();

        match (diagnostics.is_empty(), allowlist.contains(rule.code())) {
            (true, false) => problems.push(format!(
                "{} fixture {} produced no diagnostics",
                rule.code(),
                path.display()
            )),
            (false, true) => problems.push(format!(
                "{} fixture now fires; remove it from SILENT_FIXTURE_ALLOWLIST",
                rule.code()
            )),
            _ => {}
        }

        let availability = declared
            .get(&rule_variant_name(rule))
            .and_then(|impls| impls.first())
            .map_or(FixAvailability::None, |(_, availability)| *availability);
        let fixed = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.fix.is_some())
            .count();
        match availability {
            FixAvailability::None if fixed > 0 => problems.push(format!(
                "{} declares FixAvailability::None but attached {fixed} fix(es)",
                rule.code()
            )),
            FixAvailability::Always if fixed < diagnostics.len() => problems.push(format!(
                "{} declares FixAvailability::Always but {} of {} diagnostics had no fix",
                rule.code(),
                diagnostics.len() - fixed,
                diagnostics.len()
            )),
            FixAvailability::None | FixAvailability::Sometimes | FixAvailability::Always => {}
        }
    }

    assert!(
        problems.is_empty(),
        "rule fixtures are out of sync with the registry:\n{}",
        problems.join("\n"),
    );
}
