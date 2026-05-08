use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug)]
struct ForbiddenPattern {
    needle: &'static str,
    guidance: &'static str,
}

#[derive(Debug)]
struct Violation {
    path: PathBuf,
    line_number: usize,
    line: String,
    pattern: ForbiddenPattern,
}

const FORBIDDEN_PATTERNS: &[ForbiddenPattern] = &[
    ForbiddenPattern {
        needle: "source.char_indices()",
        guidance: "use RegionCollector, Locator, or LineIndex instead of rescanning the full source",
    },
    ForbiddenPattern {
        needle: "source.lines()",
        guidance: "use LineIndex line ranges instead of iterating every source line",
    },
    ForbiddenPattern {
        needle: "source.split_inclusive('\\n')",
        guidance: "use LineIndex line ranges instead of splitting the entire source",
    },
    ForbiddenPattern {
        needle: "source.split_inclusive(\"\\n\")",
        guidance: "use LineIndex line ranges instead of splitting the entire source",
    },
    ForbiddenPattern {
        needle: "while index < source.len()",
        guidance: "use parser/indexer facts instead of scanning the full source buffer",
    },
    ForbiddenPattern {
        needle: "Position::new().advanced_by(&source[..",
        guidance: "use Locator or LineIndex-backed offset-to-position helpers instead of rescanning prefixes",
    },
];

#[test]
fn facts_do_not_reintroduce_full_source_traversals() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scan_root = manifest_dir.join("src/facts");
    let mut violations = Vec::new();

    collect_violations(manifest_dir, &scan_root, &mut violations);

    assert!(
        violations.is_empty(),
        "{}",
        format_violations(manifest_dir, &violations)
    );
}

fn collect_violations(manifest_dir: &Path, path: &Path, violations: &mut Vec<Violation>) {
    let metadata = fs::metadata(path).expect("expected scan path metadata");
    if metadata.is_dir() {
        let mut entries = fs::read_dir(path)
            .expect("expected scan directory")
            .map(|entry| entry.expect("expected directory entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for entry in entries {
            collect_violations(manifest_dir, &entry, violations);
        }
        return;
    }

    if !should_scan(path) {
        return;
    }

    let source = fs::read_to_string(path).expect("expected Rust source file");
    violations.extend(file_violations(manifest_dir, path, &source));
}

fn should_scan(path: &Path) -> bool {
    if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
        return false;
    }

    !path
        .components()
        .any(|component| component.as_os_str() == "tests")
        && path.file_name().and_then(|name| name.to_str()) != Some("tests.rs")
}

fn file_violations(manifest_dir: &Path, path: &Path, source: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut brace_depth = 0i32;
    let mut pending_cfg_test_block = false;
    let mut ignored_test_depth = None::<i32>;

    for (line_index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        let opens = line.matches('{').count() as i32;
        let closes = line.matches('}').count() as i32;

        if ignored_test_depth.is_none() && trimmed.starts_with("#[cfg") && trimmed.contains("test")
        {
            pending_cfg_test_block = true;
        }

        let ignore_line = ignored_test_depth.is_some() || pending_cfg_test_block;
        if !ignore_line {
            for pattern in FORBIDDEN_PATTERNS {
                if line.contains(pattern.needle) {
                    violations.push(Violation {
                        path: path
                            .strip_prefix(manifest_dir)
                            .unwrap_or(path)
                            .to_path_buf(),
                        line_number: line_index + 1,
                        line: line.trim().to_owned(),
                        pattern: *pattern,
                    });
                }
            }
        }

        if pending_cfg_test_block {
            if opens > closes {
                ignored_test_depth = Some(brace_depth + (opens - closes));
                pending_cfg_test_block = false;
            } else if opens > 0 || trimmed.ends_with(';') {
                pending_cfg_test_block = false;
            }
        }

        brace_depth += opens - closes;
        if let Some(depth) = ignored_test_depth
            && brace_depth < depth
        {
            ignored_test_depth = None;
        }
    }

    violations
}

fn format_violations(manifest_dir: &Path, violations: &[Violation]) -> String {
    let mut message = String::from(
        "found full-source traversal patterns in shuck-linter facts; route these through parser/indexer facts instead:\n",
    );

    for violation in violations {
        let path = violation
            .path
            .strip_prefix(manifest_dir)
            .unwrap_or(&violation.path);
        message.push_str(&format!(
            "{}:{}: `{}` -> {}\n  {}\n",
            path.display(),
            violation.line_number,
            violation.pattern.needle,
            violation.pattern.guidance,
            violation.line
        ));
    }

    message
}
