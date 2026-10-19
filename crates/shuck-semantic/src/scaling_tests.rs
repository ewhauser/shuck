//! Regression tests for inputs that used to crash the semantic model or scale super-linearly.
//!
//! Each test uses a generated input large enough that the old behavior (deep recursion or
//! quadratic/cubic scans) would overflow a small stack or take far too long, while the current
//! implementation finishes quickly even in debug builds.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;
use std::thread;

use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;
use tempfile::tempdir;

use super::*;

fn build_model(source: &str) -> SemanticModel {
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    SemanticModel::build(&output.file, source, &indexer)
}

fn model_at_path(path: &Path) -> SemanticModel {
    let source = fs::read_to_string(path).unwrap();
    let output = Parser::new(&source).parse().unwrap();
    let indexer = Indexer::new(&source, &output);
    let mut observer = NoopTraversalObserver;
    build_with_observer_at_path_with_resolver(
        &output.file,
        &source,
        &indexer,
        &mut observer,
        Some(path),
        None,
    )
}

/// Runs `test` on a thread with a deliberately small stack so unbounded recursion fails fast.
fn with_small_stack<T: Send + 'static>(test: impl FnOnce() -> T + Send + 'static) -> T {
    thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap()
}

fn resolved_call_count(model: &SemanticModel, name: &str) -> usize {
    let name = Name::from(name);
    model.analysis().resolved_function_call_sites(&name).count()
}

/// `f0` calls `late`; `f_i` calls `f_{i-1}`; the outermost function is called from the top
/// level either before or after `late` is defined.
fn caller_chain_script(functions: usize, call_before_late_definition: bool) -> String {
    let mut source = String::from("#!/bin/bash\nf0() { late; }\n");
    for index in 1..functions {
        writeln!(source, "f{index}() {{ f{}; }}", index - 1).unwrap();
    }
    let outer_call = format!("f{}\n", functions - 1);
    if call_before_late_definition {
        source.push_str(&outer_call);
        source.push_str("late() { :; }\n");
    } else {
        source.push_str("late() { :; }\n");
        source.push_str(&outer_call);
    }
    source
}

#[test]
fn deep_caller_chain_resolution_does_not_recurse_per_caller() {
    with_small_stack(|| {
        let early = build_model(&caller_chain_script(5_000, true));
        assert_eq!(resolved_call_count(&early, "late"), 0);
        assert_eq!(resolved_call_count(&early, "f0"), 1);

        let late = build_model(&caller_chain_script(5_000, false));
        assert_eq!(resolved_call_count(&late, "late"), 1);
    });
}

#[test]
fn forward_caller_chain_with_late_calls_resolves_in_linear_passes() {
    // Every function calls a function defined after it plus a late-defined helper, so each
    // lookup asks a different "entered before offset" question about a long caller chain.
    let functions = 3_000;
    let mut source = String::from("#!/bin/bash\n");
    for index in 0..functions - 1 {
        writeln!(source, "f{index}() {{ f{}; late; }}", index + 1).unwrap();
    }
    writeln!(source, "f{}() {{ late; }}", functions - 1).unwrap();
    source.push_str("f0\nlate() { :; }\n");

    let model = build_model(&source);
    assert_eq!(resolved_call_count(&model, "late"), 0);
    assert_eq!(resolved_call_count(&model, "f1"), 1);
    assert_eq!(
        resolved_call_count(&model, &format!("f{}", functions - 1)),
        1
    );
}

#[test]
fn many_subshell_assignments_analyze_without_cubic_scans() {
    let source = "#!/bin/bash\n".to_owned() + &"(a=1; echo \"$a\")\n".repeat(3_000);
    let model = build_model(&source);
    let analysis = model.analyze_nonpersistent_assignments(&Default::default());
    // Each read sees the assignment made earlier in its own subshell, which resets the name.
    assert!(analysis.effects.is_empty());

    let source = "#!/bin/bash\n".to_owned() + &"(a=1)\n".repeat(3_000) + "echo \"$a\"\n";
    let model = build_model(&source);
    let analysis = model.analyze_nonpersistent_assignments(&Default::default());
    assert_eq!(analysis.effects.len(), 1);
    let last_subshell_assignment = source.rfind("a=1").unwrap();
    assert_eq!(
        analysis.effects[0].assignment_span.start.offset(),
        last_subshell_assignment
    );
}

#[test]
fn large_case_statements_build_a_linear_cfg() {
    let arms = 3_000;
    let mut source = String::from("#!/bin/bash\ncase \"$1\" in\n");
    for index in 0..arms {
        writeln!(source, "  a{index}) x={index} ;;").unwrap();
    }
    source.push_str("esac\necho \"$x\"\n");

    let model = build_model(&source);
    let analysis = model.analysis();
    let cfg = analysis.cfg();
    let edges = cfg
        .blocks()
        .iter()
        .map(|block| cfg.successors(block.id).len())
        .sum::<usize>();
    assert!(
        edges < 4 * arms,
        "case CFG has {edges} edges for {arms} arms"
    );
}

#[test]
fn case_continue_terminators_still_resume_matching_at_the_next_arm() {
    let source = "\
#!/bin/bash
case \"$1\" in
  a) x=1 ;;&
  b) y=1 ;;
  c) z=1 ;;
esac
";
    let model = build_model(source);
    let analysis = model.analysis();
    let cfg = analysis.cfg();
    let binding_block = |name: &str| {
        let binding = model
            .bindings()
            .iter()
            .find(|binding| binding.name == name)
            .unwrap();
        cfg.blocks()
            .iter()
            .find(|block| block.bindings.contains(&binding.id))
            .unwrap()
            .id
    };
    let x_block = binding_block("x");
    let y_block = binding_block("y");
    let z_block = binding_block("z");

    let [(dispatch, _)] = cfg.successors(x_block) else {
        panic!("`;;&` arm should continue to a single dispatch point");
    };
    let resumed = cfg
        .successors(*dispatch)
        .iter()
        .map(|(successor, _)| *successor)
        .collect::<Vec<_>>();
    assert!(resumed.contains(&y_block));
    assert!(resumed.contains(&z_block));
}

#[test]
fn deep_source_chains_stop_at_the_summary_depth_limit() {
    let temp = tempdir().unwrap();
    let files = 200;
    for index in 0..files {
        let mut source = String::from("#!/bin/bash\n");
        if index + 1 < files {
            writeln!(source, ". ./f{}.sh", index + 1).unwrap();
        }
        writeln!(source, "v{index}=1").unwrap();
        fs::write(temp.path().join(format!("f{index}.sh")), source).unwrap();
    }
    let entry = temp.path().join("f0.sh");

    let model = with_small_stack(move || model_at_path(&entry));
    let imported = |name: &str| {
        model
            .bindings()
            .iter()
            .any(|binding| binding.name == name && binding.kind == BindingKind::Imported)
    };
    assert!(imported("v1"));
    assert!(imported("v2"));
    assert!(!imported(&format!("v{}", files - 1)));
}

#[test]
fn helper_reads_skip_oversized_and_binary_files() {
    let temp = tempdir().unwrap();
    let script = temp.path().join("helper.sh");
    fs::write(&script, "#!/bin/bash\nvalue=1\n").unwrap();
    assert!(source_closure::read_helper_source_for_tests(&script).is_some());

    let binary = temp.path().join("tool");
    fs::write(&binary, b"\x7fELF\x02\x01\x01\x00\x00\x00").unwrap();
    assert!(source_closure::read_helper_source_for_tests(&binary).is_none());

    let oversized = temp.path().join("huge.sh");
    fs::write(&oversized, "#".repeat(5 * 1024 * 1024)).unwrap();
    assert!(source_closure::read_helper_source_for_tests(&oversized).is_none());
}

#[test]
fn executed_helpers_must_look_like_shell_scripts() {
    let temp = tempdir().unwrap();
    let write = |name: &str, contents: &[u8]| {
        let path = temp.path().join(name);
        fs::write(&path, contents).unwrap();
        path
    };
    let looks_like_shell = source_closure::executed_helper_looks_like_shell_script;

    assert!(looks_like_shell(&write(
        "bash-tool",
        b"#!/bin/bash\necho hi\n"
    )));
    assert!(looks_like_shell(&write(
        "env-sh",
        b"#!/usr/bin/env sh\necho hi\n"
    )));
    assert!(looks_like_shell(&write("no-shebang", b"echo hi\n")));
    assert!(!looks_like_shell(&write(
        "python-tool",
        b"#!/usr/bin/env python3\nprint()\n"
    )));
    assert!(!looks_like_shell(&write(
        "binary-tool",
        b"\x7fELF\x02\x01\x01\x00"
    )));
}

#[test]
fn analyzed_path_lookup_normalizes_parent_components() {
    let temp = tempdir().unwrap();
    let project = temp.path().join("project");
    let shared = temp.path().join("shared");
    fs::create_dir_all(&project).unwrap();
    fs::create_dir_all(&shared).unwrap();
    let helper = shared.join("helper.sh");
    fs::write(&helper, "helper_value=1\n").unwrap();

    let analyzed = FxHashSet::from_iter([fs::canonicalize(&helper).unwrap()]);
    let requested = project.join("../shared/helper.sh");
    assert!(source_closure::path_is_explicitly_analyzed_for_tests(
        &requested,
        Some(&analyzed)
    ));
    assert!(!source_closure::path_is_explicitly_analyzed_for_tests(
        &project.join("../shared/missing.sh"),
        Some(&analyzed)
    ));
}

#[test]
fn long_arithmetic_operator_chains_do_not_recurse_per_operator() {
    // Past the ~5.4k terms that overflowed the old per-operator recursion in debug builds.
    let terms = 6_000;
    // Parsing, indexing, and dropping such a deep tree still recurse, so those run on a large
    // stack; only the semantic build runs on the small one.
    let (y_reads, z_bindings) = thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let sum = vec!["y"; terms].join("+");
            let source = format!("#!/bin/bash\ny=1\necho $(({sum}))\n: $((z = y + y, z += y))\n");
            let output = Parser::new(&source).parse().unwrap();
            let indexer = Indexer::new(&source, &output);
            thread::scope(|scope| {
                thread::Builder::new()
                    .stack_size(1024 * 1024)
                    .spawn_scoped(scope, || {
                        let model = SemanticModel::build(&output.file, &source, &indexer);
                        let y_reads = model
                            .references()
                            .iter()
                            .filter(|reference| reference.name == "y")
                            .count();
                        let z_bindings = model
                            .bindings()
                            .iter()
                            .filter(|binding| {
                                binding.name == "z"
                                    && binding.kind == BindingKind::ArithmeticAssignment
                            })
                            .count();
                        (y_reads, z_bindings)
                    })
                    .unwrap()
                    .join()
                    .unwrap()
            })
        })
        .unwrap()
        .join()
        .unwrap();

    assert_eq!(y_reads, terms + 3);
    assert_eq!(z_bindings, 2);
}
