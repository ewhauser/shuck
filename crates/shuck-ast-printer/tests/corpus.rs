use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use shuck_ast_printer::to_typed_json;
use shuck_parser::parser::Parser;

const DEFAULT_CORPUS_DIR: &str = ".cache/scripts";
const CORPUS_ARCHIVE_NAME: &str = "corpus.tar.gz";

#[test]
#[ignore = "requires the gbash corpus; run `make test-corpus`"]
fn corpus_matches_gbash_typed_json() {
    let corpus_dir = corpus_dir();
    let script_paths = corpus_script_paths(&corpus_dir);

    assert!(
        !script_paths.is_empty(),
        "no corpus scripts found in {}. Run `make test-corpus` first.",
        corpus_dir.display()
    );

    let mut failures = Vec::new();

    for script_path in script_paths {
        let display_name = script_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("<unknown>")
            .to_owned();
        let expected_path = json_path_for(&script_path);

        if !expected_path.is_file() {
            failures.push(format!(
                "{display_name}: missing snapshot {}",
                expected_path.display()
            ));
            continue;
        }

        let source = match fs::read_to_string(&script_path) {
            Ok(source) => source,
            Err(err) => {
                failures.push(format!(
                    "{display_name}: failed to read source {}: {err}",
                    script_path.display()
                ));
                continue;
            }
        };

        let mut expected = match read_json(&expected_path) {
            Ok(value) => value,
            Err(err) => {
                failures.push(format!(
                    "{display_name}: failed to read snapshot {}: {err}",
                    expected_path.display()
                ));
                continue;
            }
        };
        normalize_corpus_json(&mut expected);

        let script = match Parser::new(&source).parse() {
            Ok(script) => script,
            Err(err) => {
                failures.push(format!("{display_name}: parser failed: {err}"));
                continue;
            }
        };

        let mut actual = to_typed_json(&script, &source);
        normalize_corpus_json(&mut actual);

        if expected != actual {
            failures.push(format!(
                "{display_name}: {}",
                describe_difference(&expected, &actual)
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} corpus file(s) did not match gbash typed JSON:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

fn corpus_dir() -> PathBuf {
    if let Some(path) = env::var_os("SHUCK_AST_CORPUS_DIR") {
        return PathBuf::from(path);
    }

    workspace_root().join(DEFAULT_CORPUS_DIR)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root should exist")
        .to_path_buf()
}

fn corpus_script_paths(corpus_dir: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(corpus_dir)
        .unwrap_or_else(|err| {
            panic!(
                "failed to read corpus directory {}: {err}. Run `make test-corpus` first.",
                corpus_dir.display()
            )
        })
        .map(|entry| entry.expect("directory entry should be readable").path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension().and_then(|ext| ext.to_str()) != Some("json")
                && path.file_name().and_then(|name| name.to_str()) != Some(CORPUS_ARCHIVE_NAME)
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn json_path_for(script_path: &Path) -> PathBuf {
    let file_name = script_path
        .file_name()
        .and_then(|name| name.to_str())
        .expect("script filename should be valid UTF-8");
    script_path.with_file_name(format!("{file_name}.json"))
}

fn read_json(path: &Path) -> Result<Value, String> {
    let contents = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn normalize_corpus_json(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let is_file = object.get("Type").and_then(Value::as_str) == Some("File");
            let has_gbash_name = object.get("Name").and_then(Value::as_str) == Some("gbash");
            if is_file && has_gbash_name {
                object.remove("Name");
            }

            for child in object.values_mut() {
                normalize_corpus_json(child);
            }
        }
        Value::Array(values) => {
            for child in values {
                normalize_corpus_json(child);
            }
        }
        _ => {}
    }
}

fn describe_difference(expected: &Value, actual: &Value) -> String {
    let (path, expected_value, actual_value) = first_difference(expected, actual, "$")
        .unwrap_or_else(|| ("$".to_owned(), render_value(expected), render_value(actual)));

    format!("mismatch at {path}: expected {expected_value}, actual {actual_value}")
}

fn first_difference(
    expected: &Value,
    actual: &Value,
    path: &str,
) -> Option<(String, String, String)> {
    if expected == actual {
        return None;
    }

    match (expected, actual) {
        (Value::Object(expected), Value::Object(actual)) => {
            let expected_keys = expected.keys().cloned().collect::<BTreeSet<_>>();
            let actual_keys = actual.keys().cloned().collect::<BTreeSet<_>>();

            if expected_keys != actual_keys {
                return Some((
                    path.to_owned(),
                    format!("keys {}", render_keys(&expected_keys)),
                    format!("keys {}", render_keys(&actual_keys)),
                ));
            }

            for key in expected.keys() {
                let expected_child = expected
                    .get(key)
                    .expect("expected child should exist for iterated key");
                let actual_child = actual
                    .get(key)
                    .expect("actual child should exist for iterated key");
                let child_path = format!("{path}.{key}");
                if let Some(diff) = first_difference(expected_child, actual_child, &child_path) {
                    return Some(diff);
                }
            }
            None
        }
        (Value::Array(expected), Value::Array(actual)) => {
            if expected.len() != actual.len() {
                return Some((
                    path.to_owned(),
                    format!("array len {}", expected.len()),
                    format!("array len {}", actual.len()),
                ));
            }

            for (index, (expected_child, actual_child)) in expected.iter().zip(actual).enumerate() {
                let child_path = format!("{path}[{index}]");
                if let Some(diff) = first_difference(expected_child, actual_child, &child_path) {
                    return Some(diff);
                }
            }
            None
        }
        _ => Some((
            path.to_owned(),
            render_value(expected),
            render_value(actual),
        )),
    }
}

fn render_keys(keys: &BTreeSet<String>) -> String {
    let joined = keys.iter().cloned().collect::<Vec<_>>().join(", ");
    format!("[{joined}]")
}

fn render_value(value: &Value) -> String {
    let rendered =
        serde_json::to_string(value).expect("serializing a JSON value for diagnostics should work");
    const MAX_LEN: usize = 160;
    if rendered.len() <= MAX_LEN {
        rendered
    } else {
        format!("{}...", &rendered[..MAX_LEN])
    }
}
