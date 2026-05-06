use super::scope_reads::{
    ScopeReadEvent, ScopeReadEventKind, future_reads_contain_after_until, resolved_calls_by_scope,
};
use super::*;
use crate::{FunctionScopeKind, SemanticModel, ShellDialect, dataflow};
use shuck_ast::Name;
use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;
use smallvec::smallvec;

fn model(source: &str) -> SemanticModel {
    model_with_dialect(source, ShellDialect::Bash)
}

fn model_with_dialect(source: &str, dialect: ShellDialect) -> SemanticModel {
    let output = Parser::with_dialect(source, dialect).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    SemanticModel::build(&output.file, source, &indexer)
}

fn reportable_unused_names(model: &SemanticModel) -> Vec<Name> {
    let analysis = model.analysis();
    analysis
        .unused_assignments()
        .iter()
        .filter_map(|binding| {
            let binding = model.binding(*binding);
            matches!(
                binding.kind,
                BindingKind::Assignment
                    | BindingKind::ArrayAssignment
                    | BindingKind::LoopVariable
                    | BindingKind::ReadTarget
                    | BindingKind::MapfileTarget
                    | BindingKind::PrintfTarget
                    | BindingKind::GetoptsTarget
                    | BindingKind::ArithmeticAssignment
            )
            .then_some(binding.name.clone())
        })
        .collect()
}

fn assert_unused_assignment_parity(model: &SemanticModel) {
    let analysis = model.analysis();
    let precise = analysis.unused_assignments().to_vec();
    let exact = analysis
        .dataflow()
        .unused_assignments
        .iter()
        .map(|unused| unused.binding)
        .collect::<Vec<_>>();
    assert_eq!(precise, exact);
}

fn assert_uninitialized_reference_parity(model: &SemanticModel) {
    let analysis = model.analysis();
    let precise = analysis.uninitialized_references().to_vec();
    let exact = analysis.dataflow().uninitialized_references.clone();
    assert_eq!(precise, exact);
}

fn assert_dead_code_parity(model: &SemanticModel) {
    let analysis = model.analysis();
    let precise = analysis.dead_code().to_vec();
    let exact = analysis.dataflow().dead_code.clone();
    assert_eq!(precise, exact);
}

fn binding_names(model: &SemanticModel, ids: &[BindingId]) -> Vec<String> {
    ids.iter()
        .map(|binding_id| model.binding(*binding_id).name.to_string())
        .collect()
}

fn sorted_binding_names<I>(model: &SemanticModel, ids: I) -> Vec<String>
where
    I: IntoIterator<Item = BindingId>,
{
    let mut names = ids
        .into_iter()
        .map(|binding_id| model.binding(binding_id).name.to_string())
        .collect::<Vec<_>>();
    names.sort_unstable();
    names
}

fn block_with_reference(cfg: &ControlFlowGraph, reference: ReferenceId) -> BlockId {
    cfg.blocks()
        .iter()
        .find(|block| block.references.contains(&reference))
        .map(|block| block.id)
        .expect("reference should be assigned to a CFG block")
}

#[test]
fn future_reads_contain_after_until_ignores_backwards_intervals() {
    let plan = ScopeReadPlan {
        direct_reads: DenseBitSet::new(1),
        calls: Vec::new(),
        events: vec![ScopeReadEvent {
            offset: 0,
            block: None,
            kind: ScopeReadEventKind::Direct(NameId(0)),
        }],
        is_function: false,
    };
    let transitive_reads = vec![DenseBitSet::new(1)];

    assert!(!future_reads_contain_after_until(
        ScopeId(0),
        10,
        5,
        NameId(0),
        &[plan],
        &transitive_reads,
    ));
}

#[test]
fn resolved_calls_by_scope_ignores_conditionally_installed_functions() {
    let source = "\
outer() {
  if false; then
use_flag() { printf '%s\\n' \"$flag\"; }
  fi
  flag=1
  use_flag
}
outer
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build(&output.file, source, &indexer);
    let name = Name::from("use_flag");
    let mut call_sites = FxHashMap::default();
    call_sites.insert(
        name.clone(),
        smallvec![model.call_sites_for(&name)[0].clone()],
    );
    let mut function_scopes = FxHashMap::default();
    for binding in model.function_definitions(&name) {
        if let Some(scope) = model.analysis().function_scope_for_binding(*binding) {
            function_scopes.insert(*binding, scope);
        }
    }

    let calls_by_scope = resolved_calls_by_scope(
        &call_sites,
        model.visible_function_call_bindings(),
        &function_scopes,
    );

    assert!(
        calls_by_scope.is_empty(),
        "resolved calls: {:?}",
        calls_by_scope
    );
}
#[test]
fn detects_overwritten_assignments_and_possible_uninitialized_reads() {
    let overwritten_source = "VAR=x\nVAR=y\necho $VAR\n";
    let overwritten = model(overwritten_source);
    let overwritten_analysis = overwritten.analysis();
    let dataflow = overwritten_analysis.dataflow();
    assert_eq!(dataflow.unused_assignments.len(), 1);
    assert!(matches!(
        dataflow.unused_assignments[0].reason,
        UnusedReason::Overwritten { .. }
    ));

    let partial_source = "if cond; then VAR=x; fi\necho $VAR\n";
    let partial = model(partial_source);
    let partial_analysis = partial.analysis();
    let dataflow = partial_analysis.dataflow();
    assert_eq!(dataflow.uninitialized_references.len(), 1);
    assert_eq!(
        dataflow.uninitialized_references[0].certainty,
        UninitializedCertainty::Possible
    );
}

#[test]
fn precise_uninitialized_references_match_dataflow_for_representative_cases() {
    let cases = [
        "echo $VAR\n",
        "if cond; then VAR=x; fi\necho $VAR\n",
        "f() { local VAR; echo \"$VAR\"; }\nf\n",
        "#!/bin/bash\nf() { local carrier; echo \"${!carrier}\"; }\nf\n",
        "printf '%s\\n' \"${VAR:-fallback}\" \"${MAYBE:+alt}\" \"${REQ:?missing}\" \"${INIT:=value}\"\nprintf '%s\\n' \"$INIT\"\n",
        "#!/bin/sh\nprintf '%s\\n' \"$HOME\"\n",
        "#!/bin/bash\nprintf '%s\\n' \"$RANDOM\"\n",
    ];

    for source in cases {
        let model = model(source);
        assert_uninitialized_reference_parity(&model);
    }
}

#[test]
fn precise_dead_code_matches_dataflow_for_representative_cases() {
    let cases = [
        "exit 0\necho dead\n",
        "\
if true; then
  exit 0
else
  exit 1
fi
echo unreachable
",
        "\
f() {
  return 0
  echo dead
}
f
",
    ];

    for source in cases {
        let model = model(source);
        assert_dead_code_parity(&model);
    }
}

#[test]
fn precompute_unused_assignments_skips_dataflow_for_linear_duplicate_assignments() {
    let model = model(
        "\
emoji[grinning]=1
emoji[smile]=2
",
    );
    let analysis = model.analysis();

    let precise = analysis.unused_assignments().to_vec();

    assert!(analysis.cfg.get().is_some());
    assert!(analysis.dataflow.get().is_none());
    assert_eq!(binding_names(&model, &precise), vec!["emoji", "emoji"]);

    let exact = analysis.dataflow().unused_assignment_ids().to_vec();
    assert_eq!(precise, exact);
}

#[test]
fn branch_only_duplicate_bindings_still_trigger_precise_unused_assignment_analysis() {
    let model = model(
        "\
if [ \"$ARCH\" = \"arm\" ]; then
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"x86_64\" ]; then
  LIBDIRSUFFIX=\"64\"
else
  LIBDIRSUFFIX=\"\"
fi
",
    );
    let analysis = model.analysis();

    let precise = analysis.unused_assignments().to_vec();

    assert!(analysis.model.needs_precise_unused_assignments());
    assert!(analysis.exact_variable_dataflow.get().is_some());
    assert_eq!(binding_names(&model, &precise), vec!["LIBDIRSUFFIX"; 3]);
}

#[test]
fn heuristic_unused_assignment_path_skips_exact_variable_dataflow_bundle() {
    let model = model("unused=1\n");
    let analysis = model.analysis();

    let precise = analysis.unused_assignments().to_vec();

    assert!(analysis.exact_variable_dataflow.get().is_none());
    assert!(analysis.dataflow.get().is_none());
    assert_eq!(binding_names(&model, &precise), vec!["unused"]);
}

#[test]
fn variable_dataflow_results_do_not_depend_on_query_order() {
    let source = "VAR=x\nVAR=y\necho $VAR\necho $UNDEF\n";
    let model = model(source);

    let unused_then_uninitialized = {
        let analysis = model.analysis();
        let unused = analysis.unused_assignments().to_vec();
        let uninitialized = analysis.uninitialized_references().to_vec();
        (unused, uninitialized)
    };
    let uninitialized_then_unused = {
        let analysis = model.analysis();
        let uninitialized = analysis.uninitialized_references().to_vec();
        let unused = analysis.unused_assignments().to_vec();
        (unused, uninitialized)
    };

    assert_eq!(unused_then_uninitialized.0, uninitialized_then_unused.0);
    assert_eq!(unused_then_uninitialized.1, uninitialized_then_unused.1);
}

#[test]
fn shared_exact_variable_dataflow_is_reused_across_accessors() {
    let model = model("VAR=x\nVAR=y\necho $VAR\necho $UNDEF\n");
    let analysis = model.analysis();

    assert!(analysis.exact_variable_dataflow.get().is_none());

    let unused = analysis.unused_assignments().to_vec();
    let bundle_ptr = analysis.exact_variable_dataflow() as *const ExactVariableDataflow;
    let uninitialized = analysis.uninitialized_references().to_vec();
    let reused_ptr = analysis.exact_variable_dataflow() as *const ExactVariableDataflow;

    assert_eq!(bundle_ptr, reused_ptr);
    assert_eq!(binding_names(&model, &unused), vec!["VAR"]);
    assert_eq!(
        uninitialized
            .iter()
            .map(|reference| model.reference(reference.reference).name.to_string())
            .collect::<Vec<_>>(),
        vec!["UNDEF"]
    );
}

#[test]
fn scope_summary_queries_reuse_exact_variable_dataflow_bundle() {
    let model = model(
        "\
outer=1
foo() {
  local skip=0
  inner=2
}
",
    );
    let foo_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "foo") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .unwrap();
    let analysis = model.analysis();

    assert!(analysis.exact_variable_dataflow.get().is_none());

    let root_bindings = analysis.summarize_scope_provided_bindings(ScopeId(0));
    let bundle_ptr = analysis.exact_variable_dataflow() as *const ExactVariableDataflow;
    let foo_bindings = analysis.summarize_scope_provided_bindings(foo_scope);
    let function_ptr = analysis.exact_variable_dataflow() as *const ExactVariableDataflow;
    let root_functions = analysis.summarize_scope_provided_functions(ScopeId(0));
    let reused_ptr = analysis.exact_variable_dataflow() as *const ExactVariableDataflow;

    assert_eq!(bundle_ptr, function_ptr);
    assert_eq!(bundle_ptr, reused_ptr);
    let cfg = analysis.cfg();
    let exact = analysis.exact_variable_dataflow();
    let context = model.dataflow_context(cfg);
    assert_eq!(
        analysis.scope_provided_bindings(ScopeId(0)),
        dataflow::summarize_scope_provided_bindings(&context, exact, ScopeId(0)).as_slice()
    );
    assert_eq!(
        analysis.scope_provided_bindings(foo_scope),
        dataflow::summarize_scope_provided_bindings(&context, exact, foo_scope).as_slice()
    );
    assert_eq!(
        analysis.definite_provider_scopes(&Name::from("outer")),
        &[ScopeId(0)]
    );
    assert_eq!(
        analysis.definite_provider_scopes(&Name::from("inner")),
        &[foo_scope]
    );
    assert_eq!(
        root_bindings
            .iter()
            .map(|binding| binding.name.to_string())
            .collect::<Vec<_>>(),
        vec!["outer"]
    );
    assert_eq!(
        foo_bindings
            .iter()
            .map(|binding| binding.name.to_string())
            .collect::<Vec<_>>(),
        vec!["inner"]
    );
    assert_eq!(
        root_functions
            .iter()
            .map(|binding| binding.name.to_string())
            .collect::<Vec<_>>(),
        vec!["foo"]
    );
}

#[test]
fn materialized_reaching_definitions_match_dense_exact_results() {
    let model = model("VAR=outer\nif cond; then VAR=inner; fi\necho $VAR\n");
    let analysis = model.analysis();
    let reaching_definitions = analysis.materialized_reaching_definitions();
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name.as_str() == "VAR")
        .expect("expected a VAR reference");
    let block_id = block_with_reference(analysis.cfg(), reference.id);

    assert_eq!(
        sorted_binding_names(
            &model,
            reaching_definitions.reaching_in[&block_id].iter().copied()
        ),
        vec!["VAR", "VAR"]
    );
}

#[test]
fn precise_unused_assignments_match_dataflow_for_representative_cases() {
    let cases = [
        "VAR=x\nVAR=y\necho $VAR\n",
        "\
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
${code_command} --version
",
        "\
pass_args() {
  local_install=1
  proxy=$1
}
main() {
  pass_args \"$@\"
  printf '%s %s\\n' \"$local_install\" \"$proxy\"
}
main \"$@\"
",
        "\
check_status() {
  if [[ $is_wget ]]; then
printf '%s\\n' ok
  else
is_wget=1
check_status
  fi
}
check_status
",
        "\
#!/bin/bash
IFS=$'\\n\\t'
unused=1
echo ok
",
        "\
#!/bin/bash
apache_args=(--apache)
unused_args=(--unused)
args_var=apache_args[@]
printf '%s\\n' \"${!args_var}\"
",
        "\
#!/bin/bash
apache_args=(--apache)
nginx_args=(--nginx)
apache_args+=(--common)
nginx_args+=(--common)
web_server=apache
args_var=\"${web_server}_args[@]\"
printf '%s\\n' \"${!args_var}\"
",
        "\
#!/bin/bash
f() {
  local IFS=$'\\n'
  local unused=1
  read -d '' -ra reply < <(printf 'alpha\\nbeta\\0')
  printf '%s\\n' \"${reply[@]}\"
}
f
",
    ];

    for source in cases {
        let model = model(source);
        assert_unused_assignment_parity(&model);
    }
}

#[test]
fn backward_liveness_unused_assignment_reports_only_values_that_do_not_feed_later_reads() {
    let source = "\
value=one
value=two
value=three
printf '%s\\n' \"$value\"
";
    let model = model(source);
    let unused_lines = model
        .analysis()
        .dataflow()
        .unused_assignments
        .iter()
        .map(|unused| model.binding(unused.binding).span.start.line)
        .collect::<Vec<_>>();

    assert_eq!(unused_lines, vec![1, 2]);
}

#[test]
fn backward_liveness_preserves_unused_assignment_edge_cases() {
    let cases = [
        (
            "\
#!/bin/bash
arr=(--first)
arr+=(--second)
printf '%s\\n' \"${arr[@]}\"
",
            "arr",
        ),
        (
            "\
reader() {
  printf '%s\\n' \"$value\"
}
main() {
  value=ok
  reader
}
main
",
            "value",
        ),
        (
            "\
f() {
  foo=1
  if cond; then
local foo
  fi
  echo \"$foo\"
}
f
",
            "foo",
        ),
    ];

    for (source, live_name) in cases {
        let model = model(source);
        let unused = binding_names(&model, model.analysis().unused_assignments());
        assert!(
            !unused.iter().any(|name| name == live_name),
            "unused bindings for {live_name}: {unused:?}"
        );
    }
}

#[test]
fn branch_assignments_reaching_a_later_read_are_both_used() {
    let source = "\
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
${code_command} --version
";
    let model = model(source);
    let analysis = model.analysis();
    let dataflow = analysis.dataflow();

    assert!(dataflow.unused_assignments.is_empty());
}

#[test]
fn mutually_exclusive_unused_branch_assignments_collapse_to_one_reported_id() {
    let source = "\
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
";
    let model = model(source);
    let all_bindings = model.bindings_for(&Name::from("code_command")).to_vec();
    let binding_ids = model.analysis().dataflow().unused_assignment_ids().to_vec();

    assert_eq!(model.analysis().dataflow().unused_assignments.len(), 2);
    assert_eq!(binding_ids, vec![all_bindings[1]]);
}

#[test]
fn public_unused_assignments_keep_all_dead_branch_family_members() {
    let source = "\
if [ \"$ARCH\" = \"arm\" ]; then
  LIBDIRSUFFIX=\"\"
elif [ \"$ARCH\" = \"x86_64\" ]; then
  LIBDIRSUFFIX=\"64\"
else
  LIBDIRSUFFIX=\"\"
fi
";
    let model = model(source);
    let all_bindings = model.bindings_for(&Name::from("LIBDIRSUFFIX")).to_vec();
    let precise = model.analysis().unused_assignments().to_vec();
    let collapsed = model.analysis().dataflow().unused_assignment_ids().to_vec();

    assert_eq!(precise, all_bindings);
    assert_eq!(collapsed, vec![all_bindings[2]]);
}

#[test]
fn partially_used_branch_assignments_keep_each_dead_arm_reported() {
    let source = "\
if a; then
  VAR=1
elif b; then
  VAR=2
else
  VAR=3
  echo \"$VAR\"
fi
";
    let model = model(source);
    let all_bindings = model.bindings_for(&Name::from("VAR")).to_vec();
    let binding_ids = model.analysis().dataflow().unused_assignment_ids().to_vec();

    assert_eq!(binding_ids, vec![all_bindings[0], all_bindings[1]]);
}

#[test]
fn used_uninitialized_local_branches_keep_each_dead_arm_reported() {
    let source = "\
f() {
  if a; then
foo=1
  elif b; then
local foo
echo \"$foo\"
  else
foo=3
  fi
}
f
";
    let model = model(source);
    let assignment_bindings = model
        .bindings_for(&Name::from("foo"))
        .iter()
        .copied()
        .filter(|binding_id| matches!(model.binding(*binding_id).kind, BindingKind::Assignment))
        .collect::<Vec<_>>();
    let binding_ids = model.analysis().dataflow().unused_assignment_ids().to_vec();

    assert_eq!(binding_ids, assignment_bindings);
}

#[test]
fn unused_uninitialized_local_branches_do_not_hide_dead_assignments() {
    let source = "\
f() {
  if a; then
foo=1
  else
local foo
  fi
}
f
";
    let model = model(source);
    let assignment_binding = model
        .bindings_for(&Name::from("foo"))
        .iter()
        .copied()
        .find(|binding_id| matches!(model.binding(*binding_id).kind, BindingKind::Assignment))
        .expect("expected assignment binding");
    let binding_ids = model.analysis().dataflow().unused_assignment_ids().to_vec();

    assert!(binding_ids.contains(&assignment_binding));
}

#[test]
fn branch_local_uninitialized_declarations_preserve_other_reaching_defs() {
    let source = "\
f() {
  foo=1
  if cond; then
local foo
  fi
  echo \"$foo\"
}
f
";
    let model = model(source);
    let unused = model.analysis().dataflow().unused_assignment_ids().to_vec();

    assert!(
        unused.is_empty(),
        "unused: {:?}",
        unused
            .iter()
            .map(|binding| {
                let binding = model.binding(*binding);
                (binding.name.to_string(), binding.span.start.line)
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn branch_local_declarations_do_not_hide_dynamic_scope_reads_in_called_functions() {
    let source = "\
g() {
  echo \"$foo\"
}
f() {
  foo=1
  if cond; then
local foo
  fi
  g
}
f
";
    let model = model(source);
    let unused = model.analysis().dataflow().unused_assignment_ids().to_vec();

    assert!(
        unused.is_empty(),
        "unused: {:?}",
        unused
            .iter()
            .map(|binding| {
                let binding = model.binding(*binding);
                (binding.name.to_string(), binding.span.start.line)
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn linear_duplicate_assignments_with_unrelated_reads_keep_all_reported_ids() {
    let source = "\
emoji[grinning]=1
printf '%s\n' \"$OTHER\"
emoji[smile]=2
";
    let model = model(source);
    let all_bindings = model.bindings_for(&Name::from("emoji")).to_vec();
    let binding_ids = model.analysis().dataflow().unused_assignment_ids().to_vec();

    assert_eq!(binding_ids, all_bindings);
}

#[test]
fn branch_join_defs_used_in_later_function_body_are_all_live() {
    let source = "\
if command -v code >/dev/null 2>&1; then
  code_command=\"code\"
else
  code_command=\"flatpak run com.visualstudio.code\"
fi
show_version() { ${code_command} --version; }
";
    let model = model(source);
    let unused_bindings = model
        .analysis()
        .dataflow()
        .unused_assignments
        .iter()
        .map(|unused| unused.binding)
        .collect::<Vec<_>>();
    let unused_names = unused_bindings
        .into_iter()
        .map(|binding| model.binding(binding).name.to_string())
        .collect::<Vec<_>>();

    assert!(!unused_names.contains(&"code_command".to_string()));
}

#[test]
fn elif_branch_join_defs_used_in_later_function_body_are_all_live() {
    let source = "\
if [ \"$arch\" = amd64 ]; then
  jq_arch=amd64
elif [ \"$arch\" = arm64 ]; then
  jq_arch=arm64
else
  jq_arch=unknown
fi
download() { echo \"$jq_arch\"; }
";
    let model = model(source);
    let unused_bindings = model
        .analysis()
        .dataflow()
        .unused_assignments
        .iter()
        .map(|unused| unused.binding)
        .collect::<Vec<_>>();
    let unused_names = unused_bindings
        .into_iter()
        .map(|binding| model.binding(binding).name.to_string())
        .collect::<Vec<_>>();

    assert!(!unused_names.contains(&"jq_arch".to_string()));
}

#[test]
fn case_branch_join_defs_used_in_later_function_body_are_all_live() {
    let source = "\
case \"$arch\" in
amd64 | x86_64)
  jq_arch=amd64
  core_arch=64
  ;;
arm64 | aarch64)
  jq_arch=arm64
  core_arch=arm64-v8a
  ;;
esac
download() {
  echo \"$jq_arch\"
  echo \"$core_arch\"
}
";
    let model = model(source);
    let unused_bindings = model
        .analysis()
        .dataflow()
        .unused_assignments
        .iter()
        .map(|unused| unused.binding)
        .collect::<Vec<_>>();
    let unused_names = unused_bindings
        .into_iter()
        .map(|binding| model.binding(binding).name.to_string())
        .collect::<Vec<_>>();

    assert!(!unused_names.contains(&"jq_arch".to_string()));
    assert!(!unused_names.contains(&"core_arch".to_string()));
}

#[test]
fn case_without_matching_arm_keeps_initializer_live() {
    let source = "\
value=''
case \"$kind\" in
  one)
value=1
;;
  two)
value=2
;;
esac
printf '%s\\n' \"$value\"
";
    let model = model_with_dialect(source, ShellDialect::Posix);
    let unused = reportable_unused_names(&model);

    assert!(
        !unused.contains(&Name::from("value")),
        "unused bindings: {:?}",
        unused
    );
}

#[test]
fn case_with_catch_all_arm_overwrites_initializer() {
    let source = "\
value=''
case \"$kind\" in
  one)
value=1
;;
  *)
value=2
;;
esac
printf '%s\\n' \"$value\"
";
    let model = model_with_dialect(source, ShellDialect::Posix);
    let unused = reportable_unused_names(&model);
    let count = unused
        .iter()
        .filter(|name| name.as_str() == "value")
        .count();

    assert_eq!(count, 1, "unused bindings: {:?}", unused);
}

#[test]
fn conditional_function_install_is_not_a_visible_function_call_binding() {
    let source = "\
outer() {
  if false; then
use_flag() { printf '%s\\n' \"$flag\"; }
  fi
  flag=1
  use_flag
}
outer
";
    let model = model(source);
    let name = Name::from("use_flag");
    let call = &model.call_sites_for(&name)[0];

    assert_eq!(
        model
            .analysis()
            .visible_function_binding_at_call(&name, call.name_span),
        None
    );
}

#[test]
fn empty_case_catch_all_arm_keeps_following_code_reachable() {
    let source = "\
case \"$kind\" in
  *)
;;
esac
printf '%s\\n' ok
";
    let model = model_with_dialect(source, ShellDialect::Posix);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn catch_all_continue_case_arm_keeps_following_code_reachable() {
    let source = "\
case \"$kind\" in
  *)
:
;;&
esac
printf '%s\\n' ok
";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn function_global_assignments_read_later_by_caller_are_live() {
    let source = "\
pass_args() {
  local_install=1
  proxy=$1
}
main() {
  pass_args \"$@\"
  printf '%s %s\\n' \"$local_install\" \"$proxy\"
}
main \"$@\"
";
    let model = model(source);
    let unused_bindings = model
        .analysis()
        .dataflow()
        .unused_assignments
        .iter()
        .map(|unused| unused.binding)
        .collect::<Vec<_>>();
    let unused_names = unused_bindings
        .into_iter()
        .map(|binding| model.binding(binding).name.to_string())
        .collect::<Vec<_>>();

    assert!(!unused_names.contains(&"local_install".to_string()));
    assert!(!unused_names.contains(&"proxy".to_string()));
}

#[test]
fn callee_subshell_reads_keep_caller_assignments_live() {
    let source = "\
#!/bin/bash
install_package() {
  (
printf '%s\\n' \"$archive_format\" \"${configure[@]}\"
  )
}
install_readline() {
  archive_format='tar.gz'
  configure=( ./configure --disable-dependency-tracking )
  install_package
}
install_readline
";
    let model = model(source);
    let unused = reportable_unused_names(&model);

    assert!(
        !unused.contains(&Name::from("archive_format")),
        "unused: {:?}",
        unused
    );
    assert!(
        !unused.contains(&Name::from("configure")),
        "unused: {:?}",
        unused
    );
}

#[test]
fn later_file_scope_helper_reads_keep_caller_local_assignment_live() {
    let source = "\
main() {
  local status=''
  helper
  printf '%s\\n' \"$status\"
}
helper() {
  status=ok
}
main
";
    let model = model(source);

    let unused = reportable_unused_names(&model);
    assert!(
        !unused.contains(&Name::from("status")),
        "unused: {:?}",
        unused
    );
}

#[test]
fn later_file_scope_helper_appends_keep_caller_local_array_live() {
    let source = "\
#!/bin/bash
main() {
  local errors=()
  helper
  printf '%s\\n' \"${errors[@]}\"
}
helper() {
  errors+=(oops)
}
main
";
    let model = model(source);

    let unused = reportable_unused_names(&model);
    assert!(
        !unused.contains(&Name::from("errors")),
        "unused: {:?}",
        unused
    );
}

#[test]
fn recursive_function_reads_keep_later_global_write_live() {
    let source = "\
check_status() {
  if [[ $is_wget ]]; then
printf '%s\\n' ok
  else
is_wget=1
check_status
  fi
}
check_status
";
    let model = model(source);
    let unused_bindings = model
        .analysis()
        .dataflow()
        .unused_assignments
        .iter()
        .map(|unused| unused.binding)
        .collect::<Vec<_>>();
    let unused_names = unused_bindings
        .into_iter()
        .map(|binding| model.binding(binding).name.to_string())
        .collect::<Vec<_>>();

    assert!(!unused_names.contains(&"is_wget".to_string()));
}
