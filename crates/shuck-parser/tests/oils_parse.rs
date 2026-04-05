use std::{
    collections::{BTreeSet, HashMap},
    fs,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
};

use serde::Deserialize;
use shuck_parser::parser::Parser;

const OILS_DIR: &str = "tests/testdata/oils";
const EXPECTATIONS_PATH: &str = "tests/testdata/oils_expectations.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Expectation {
    ParseOk,
    ParseErr,
    Skip,
}

#[derive(Debug, Clone, Deserialize)]
struct ExpectationEntry {
    expectation: Expectation,
    reason: String,
}

#[derive(Debug, Clone)]
struct SpecCase {
    name: String,
    script: String,
}

#[derive(Debug, Clone)]
struct SpecFile {
    path: String,
    cases: Vec<SpecCase>,
}

#[test]
fn oils_corpus_matches_parser_expectations() {
    let oils_dir = manifest_dir().join(OILS_DIR);
    let expectations_path = manifest_dir().join(EXPECTATIONS_PATH);
    let spec_files = load_spec_files(&oils_dir);
    let expectations = load_expectations(&expectations_path);
    validate_expectations(&expectations, &spec_files);

    let mut failures = Vec::new();
    let mut total_cases = 0usize;
    let mut skipped_cases = 0usize;

    for spec_file in &spec_files {
        for spec_case in &spec_file.cases {
            total_cases += 1;
            let case_key = format!("{}::{}", spec_file.path, spec_case.name);
            let expectation = expectation_for(&expectations, &spec_file.path, &spec_case.name);

            if expectation == Expectation::Skip {
                skipped_cases += 1;
                continue;
            }

            let outcome = panic::catch_unwind(AssertUnwindSafe(|| Parser::new(&spec_case.script).parse()));
            match (expectation, outcome) {
                (Expectation::ParseOk, Ok(Ok(_))) => {}
                (Expectation::ParseErr, Ok(Err(_))) => {}
                (Expectation::ParseOk, Ok(Err(err))) => failures.push(format!(
                    "{case_key}: unexpected parse error: {err}"
                )),
                (Expectation::ParseErr, Ok(Ok(_))) => {
                    failures.push(format!("{case_key}: unexpected parse success"))
                }
                (_, Err(_)) => failures.push(format!("{case_key}: parser panic")),
                (Expectation::Skip, _) => unreachable!("skipped cases return early"),
            }
        }
    }

    assert!(
        failures.is_empty(),
        "OILS parser corpus had {} failure(s) across {} cases ({} skipped):\n\n{}",
        failures.len(),
        total_cases,
        skipped_cases,
        failures.join("\n\n")
    );
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn load_spec_files(oils_dir: &Path) -> Vec<SpecFile> {
    let mut paths = fs::read_dir(oils_dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", oils_dir.display()))
        .map(|entry| entry.expect("fixture directory entry should be readable").path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("sh"))
        .collect::<Vec<_>>();
    paths.sort();

    assert!(
        !paths.is_empty(),
        "no OILS spec fixtures found in {}",
        oils_dir.display()
    );

    paths.into_iter()
        .map(|path| {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
            parse_spec_file(&path, &source)
        })
        .collect()
}

fn parse_spec_file(path: &Path, source: &str) -> SpecFile {
    let rel_path = format!(
        "oils/{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .expect("fixture filename should be valid UTF-8")
    );

    let mut cases = Vec::new();
    let mut current_name: Option<String> = None;
    let mut script_lines = Vec::new();
    let mut in_block_directive = false;
    let flush_case = |cases: &mut Vec<SpecCase>,
                      current_name: &mut Option<String>,
                      script_lines: &mut Vec<String>|
     -> bool {
        let Some(name) = current_name.take() else {
            return false;
        };
        let mut script = script_lines.join("\n");
        if !script_lines.is_empty() {
            script.push('\n');
        }
        script_lines.clear();
        cases.push(SpecCase { name, script });
        true
    };

    for raw_line in source.lines() {
        if let Some(name) = raw_line.strip_prefix("#### ") {
            flush_case(&mut cases, &mut current_name, &mut script_lines);
            current_name = Some(name.trim().to_owned());
            in_block_directive = false;
            continue;
        }

        if current_name.is_none() {
            continue;
        }

        let trimmed = raw_line.trim();

        if in_block_directive {
            if trimmed == "## END" {
                in_block_directive = false;
                continue;
            }
            if raw_line.starts_with("##") {
                in_block_directive = false;
            } else {
                continue;
            }
        }

        if raw_line.starts_with("##") {
            if is_directive_block_header(raw_line) {
                in_block_directive = true;
            }
            continue;
        }

        script_lines.push(raw_line.to_owned());
    }

    flush_case(&mut cases, &mut current_name, &mut script_lines);

    assert!(!cases.is_empty(), "no cases found in {}", rel_path);
    SpecFile {
        path: rel_path,
        cases,
    }
}

fn is_directive_block_header(line: &str) -> bool {
    let body = line.trim_start().trim_start_matches("##").trim();
    body.ends_with("STDOUT:") || body.ends_with("STDERR:")
}

fn load_expectations(path: &Path) -> HashMap<String, ExpectationEntry> {
    let contents =
        fs::read_to_string(path).unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    serde_json::from_str(&contents)
        .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()))
}

fn validate_expectations(
    expectations: &HashMap<String, ExpectationEntry>,
    spec_files: &[SpecFile],
) {
    let mut known_keys = BTreeSet::new();
    for spec_file in spec_files {
        known_keys.insert(spec_file.path.clone());
        for spec_case in &spec_file.cases {
            known_keys.insert(format!("{}::{}", spec_file.path, spec_case.name));
        }
    }

    let unknown = expectations
        .keys()
        .filter(|key| !known_keys.contains(*key))
        .cloned()
        .collect::<Vec<_>>();

    assert!(
        unknown.is_empty(),
        "unknown OILS expectation key(s): {}",
        unknown.join(", ")
    );

    for (key, entry) in expectations {
        assert!(
            !entry.reason.trim().is_empty(),
            "expectation {key} must have a non-empty reason"
        );
    }
}

fn expectation_for(
    expectations: &HashMap<String, ExpectationEntry>,
    file_path: &str,
    case_name: &str,
) -> Expectation {
    let case_key = format!("{file_path}::{case_name}");
    expectations
        .get(&case_key)
        .or_else(|| expectations.get(file_path))
        .map(|entry| entry.expectation)
        .unwrap_or(Expectation::ParseOk)
}
