use super::*;
use crate::cfg::build_control_flow_graph;
use shuck_ast::{Command, CompoundCommand, Position, Span};
use shuck_indexer::Indexer;
use shuck_parser::parser::{Parser, ShellDialect};
use std::fs;
use std::path::Path;
use tempfile::tempdir;

fn model(source: &str) -> SemanticModel {
    model_with_dialect(source, ShellDialect::Bash)
}

fn model_with_dialect(source: &str, dialect: ShellDialect) -> SemanticModel {
    let output = Parser::with_dialect(source, dialect).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    SemanticModel::build(&output.file, source, &indexer)
}

fn model_with_profile(source: &str, profile: ShellProfile) -> SemanticModel {
    let output = Parser::with_profile(source, profile.clone())
        .parse()
        .unwrap();
    let indexer = Indexer::new(source, &output);
    SemanticModel::build_with_options(
        &output.file,
        source,
        &indexer,
        SemanticBuildOptions {
            shell_profile: Some(profile),
            ..SemanticBuildOptions::default()
        },
    )
}

fn span_for_nth(source: &str, needle: &str, index: usize) -> Span {
    let start_offset = source
        .match_indices(needle)
        .nth(index)
        .map(|(offset, _)| offset)
        .unwrap();
    let start = Position::new().advanced_by(&source[..start_offset]);
    Span::from_positions(start, start.advanced_by(needle))
}

fn function_binding_id(model: &SemanticModel, name: &str, index: usize) -> BindingId {
    model.function_definitions(&Name::from(name))[index]
}

fn command_id_starting_with(
    model: &SemanticModel,
    source: &str,
    prefix: &str,
) -> Option<CommandId> {
    model.commands().iter().copied().find(|id| {
        model
            .command_syntax_span(*id)
            .slice(source)
            .starts_with(prefix)
    })
}

fn command_id_containing(model: &SemanticModel, source: &str, needle: &str) -> Option<CommandId> {
    model.commands().iter().copied().find(|id| {
        model
            .command_syntax_span(*id)
            .slice(source)
            .contains(needle)
    })
}

fn model_at_path_with_parse_dialect(path: &Path, dialect: ShellDialect) -> SemanticModel {
    let source = fs::read_to_string(path).unwrap();
    let output = Parser::with_dialect(&source, dialect).parse().unwrap();
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

fn model_at_path(path: &Path) -> SemanticModel {
    model_at_path_with_resolver(path, None)
}

fn model_at_path_with_resolver(
    path: &Path,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> SemanticModel {
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
        source_path_resolver,
    )
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

fn binding_for_name<'a>(model: &'a SemanticModel, name: &str) -> &'a Binding {
    let ids = model.bindings_for(&Name::from(name));
    assert_eq!(ids.len(), 1, "expected one binding for {name}, got {ids:?}");
    model.binding(ids[0])
}

fn block_with_reference(cfg: &ControlFlowGraph, reference: ReferenceId) -> BlockId {
    cfg.blocks()
        .iter()
        .find(|block| block.references.contains(&reference))
        .map(|block| block.id)
        .expect("reference should be assigned to a CFG block")
}

fn unresolved_names(model: &SemanticModel) -> Vec<String> {
    model
        .unresolved_references()
        .iter()
        .map(|reference| model.reference(*reference).name.to_string())
        .collect()
}

fn uninitialized_names(model: &SemanticModel) -> Vec<String> {
    let analysis = model.analysis();
    let references = analysis
        .uninitialized_references()
        .iter()
        .map(|reference| reference.reference)
        .collect::<Vec<_>>();
    references
        .iter()
        .map(|reference| model.reference(*reference).name.to_string())
        .collect()
}

fn uninitialized_details(model: &SemanticModel) -> Vec<(String, UninitializedCertainty)> {
    let references = model.analysis().uninitialized_references().to_vec();
    references
        .iter()
        .map(|reference| {
            (
                model.reference(reference.reference).name.to_string(),
                reference.certainty,
            )
        })
        .collect()
}

fn assert_names_absent(names: &[&str], actual: &[String]) {
    for name in names {
        assert!(
            !actual.iter().any(|actual_name| actual_name == name),
            "did not expect `{name}` in {actual:?}"
        );
    }
}

fn assert_names_present(names: &[&str], actual: &[String]) {
    for name in names {
        assert!(
            actual.iter().any(|actual_name| actual_name == name),
            "expected `{name}` in {actual:?}"
        );
    }
}

fn arithmetic_read_count(model: &SemanticModel, name: &str) -> usize {
    model
        .references()
        .iter()
        .filter(|reference| {
            reference.kind == ReferenceKind::ArithmeticRead && reference.name == name
        })
        .count()
}

fn arithmetic_write_count(model: &SemanticModel, name: &str) -> usize {
    model
        .bindings()
        .iter()
        .filter(|binding| binding.kind == BindingKind::ArithmeticAssignment && binding.name == name)
        .count()
}

fn assert_arithmetic_usage(
    model: &SemanticModel,
    name: &str,
    expected_reads: usize,
    expected_writes: usize,
) {
    assert_eq!(
        arithmetic_read_count(model, name),
        expected_reads,
        "unexpected arithmetic read count for {name}"
    );
    assert_eq!(
        arithmetic_write_count(model, name),
        expected_writes,
        "unexpected arithmetic write count for {name}"
    );
}

fn common_runtime_source(shebang: &str) -> String {
    format!(
        "{shebang}\nprintf '%s\\n' \"$IFS\" \"$USER\" \"$HOME\" \"$SHELL\" \"$PWD\" \"$TERM\" \"$PATH\" \"$CDPATH\" \"$LANG\" \"$LC_ALL\" \"$LC_TIME\" \"$SUDO_USER\" \"$DOAS_USER\"\n"
    )
}

fn bash_runtime_source(shebang: &str) -> String {
    format!(
        "{shebang}\nprintf '%s\\n' \"$LINENO\" \"$FUNCNAME\" \"${{BASH_SOURCE[0]}}\" \"${{BASH_LINENO[0]}}\" \"$RANDOM\" \"${{BASH_REMATCH[0]}}\" \"$READLINE_LINE\" \"$BASH_VERSION\" \"${{BASH_VERSINFO[0]}}\" \"$OSTYPE\" \"$HISTCONTROL\" \"$HISTSIZE\"\n"
    )
}

fn zsh_runtime_source(shebang: &str) -> String {
    format!(
        "{shebang}\nprintf '%s\\n' \"${{options[xtrace]}}\" \"${{functions[typeset]}}\" \"${{aliases[ls]}}\" \"${{commands[printf]}}\" \"${{parameters[path]}}\" \"${{termcap[ku]}}\" \"${{terminfo[kcuu1]}}\" \"${{path[1]}}\" \"${{pipestatus[1]}}\" \"${{funcstack[1]}}\" \"${{funcfiletrace[1]}}\" \"${{funcsourcetrace[1]}}\" \"${{psvar[1]}}\" \"${{widgets[widget]}}\" \"${{zsh_eval_context[1]}}\" \"${{module_path[1]}}\" \"${{manpath[1]}}\" \"${{mailpath[1]}}\" \"${{historywords[1]}}\" \"${{jobdirs[1]}}\" \"${{jobstates[1]}}\" \"${{jobtexts[1]}}\" \"${{signals[1]}}\" \"$MATCH\" \"$MBEGIN\" \"$MEND\" \"$BUFFER\" \"$LBUFFER\" \"$RBUFFER\" \"$CURSOR\" \"$WIDGET\" \"$KEYS\" \"$NUMERIC\" \"$POSTDISPLAY\" \"$region_highlight\" \"$LINES\" \"$COLUMNS\" \"$ZSH_VERSION\" \"$ZSH_NAME\" \"$ZSH_PATCHLEVEL\" \"$ZSH_SUBSHELL\" \"$ZSH_ARGZERO\"\n"
    )
}

#[test]
fn creates_file_and_function_scopes_and_resolves_local_shadowing() {
    let source = "VAR=global\nf() { local VAR=local; echo $VAR; }\n";
    let model = model(source);

    assert!(matches!(model.scope_kind(ScopeId(0)), ScopeKind::File));
    assert!(model.scopes().iter().any(|scope| {
        matches!(
            &scope.kind,
            ScopeKind::Function(function) if function.contains_name_str("f")
        )
    }));

    let local_binding = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "VAR"
                && matches!(
                    binding.kind,
                    BindingKind::Declaration(DeclarationBuiltin::Local)
                )
        })
        .unwrap();
    assert!(matches!(
        model.scope_kind(local_binding.scope),
        ScopeKind::Function(function) if function.contains_name_str("f")
    ));

    let reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "VAR")
        .unwrap();
    let resolved = model.resolved_binding(reference.id).unwrap();
    assert_eq!(resolved.id, local_binding.id);
}

#[test]
fn brace_fd_redirect_target_resolves_before_new_fd_binding() {
    let source = "\
#!/bin/bash
fd='1 2'
exec {fd}>&$fd
printf '%s\\n' \"$fd\"
";
    let model = model(source);

    let fd_bindings = model.bindings_for(&Name::from("fd"));
    assert_eq!(
        fd_bindings.len(),
        2,
        "expected scalar and brace-fd bindings"
    );
    let initial_binding = model.binding(fd_bindings[0]);
    let brace_fd_binding = model.binding(fd_bindings[1]);
    assert!(
        brace_fd_binding
            .attributes
            .contains(BindingAttributes::INTEGER)
    );

    let fd_refs = model
        .references()
        .iter()
        .filter(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "fd")
        .collect::<Vec<_>>();
    assert_eq!(fd_refs.len(), 2);

    let redirect_target_ref = fd_refs
        .iter()
        .find(|reference| reference.span.start.line == 3)
        .unwrap();
    let later_ref = fd_refs
        .iter()
        .find(|reference| reference.span.start.line == 4)
        .unwrap();

    assert_eq!(
        model.resolved_binding(redirect_target_ref.id).unwrap().id,
        initial_binding.id
    );
    assert_eq!(
        model.resolved_binding(later_ref.id).unwrap().id,
        brace_fd_binding.id
    );
}

#[test]
fn declare_plus_g_stays_local_inside_functions() {
    let source = "\
#!/bin/bash
f() {
  declare +g scoped=1
  printf '%s\\n' \"$scoped\"
}
printf '%s\\n' \"$scoped\"
";
    let model = model(source);

    let scoped_binding = binding_for_name(&model, "scoped");
    let ScopeKind::Function(function_scope) = model.scope_kind(scoped_binding.scope) else {
        panic!("expected declare +g binding to live in the function scope");
    };
    assert!(function_scope.contains_name_str("f"));

    let scoped_refs = model
        .references()
        .iter()
        .filter(|reference| {
            reference.kind == ReferenceKind::Expansion && reference.name == "scoped"
        })
        .collect::<Vec<_>>();
    assert_eq!(scoped_refs.len(), 2);

    let inner_ref = scoped_refs
        .iter()
        .find(|reference| reference.span.start.line == 4)
        .unwrap();
    let outer_ref = scoped_refs
        .iter()
        .find(|reference| reference.span.start.line == 6)
        .unwrap();

    assert_eq!(
        model.resolved_binding(inner_ref.id).unwrap().id,
        scoped_binding.id
    );
    assert!(model.resolved_binding(outer_ref.id).is_none());
}

#[test]
fn analysis_maps_function_binding_to_body_scope() {
    let model = model(
        "\
x=1
f() {
  :
}
",
    );
    let function_binding = binding_for_name(&model, "f");
    let assignment_binding = binding_for_name(&model, "x");
    let analysis = model.analysis();

    let function_scope = analysis
        .function_scope_for_binding(function_binding.id)
        .expect("expected function body scope for function binding");
    let ScopeKind::Function(function_scope_kind) = model.scope_kind(function_scope) else {
        panic!("expected function binding to map to a function scope");
    };
    assert!(function_scope_kind.contains_name_str("f"));
    assert_eq!(
        analysis.function_scope_for_binding(assignment_binding.id),
        None
    );
}

#[test]
fn zsh_anonymous_functions_create_function_scoped_locals() {
    let source = "function { local scoped=1; echo \"$scoped\" \"$1\"; } arg\necho \"$scoped\"\n";
    let model = model_with_dialect(source, ShellDialect::Zsh);

    let local_binding = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "scoped"
                && matches!(
                    binding.kind,
                    BindingKind::Declaration(DeclarationBuiltin::Local)
                )
        })
        .unwrap();
    let ScopeKind::Function(function_scope) = model.scope_kind(local_binding.scope) else {
        panic!("expected local binding to live in a function scope");
    };
    assert!(function_scope.is_anonymous());

    let scoped_refs = model
        .references()
        .iter()
        .filter(|reference| {
            reference.kind == ReferenceKind::Expansion && reference.name == "scoped"
        })
        .collect::<Vec<_>>();
    assert_eq!(scoped_refs.len(), 2);

    let inner_ref = scoped_refs
        .iter()
        .find(|reference| reference.span.start.line == 1)
        .unwrap();
    let outer_ref = scoped_refs
        .iter()
        .find(|reference| reference.span.start.line == 2)
        .unwrap();

    assert_eq!(
        model.resolved_binding(inner_ref.id).unwrap().id,
        local_binding.id
    );
    assert!(model.resolved_binding(outer_ref.id).is_none());
}

#[test]
fn zsh_multi_name_functions_bind_each_static_alias() {
    let source = "function music itunes() { local track=1; }\n";
    let model = model_with_dialect(source, ShellDialect::Zsh);

    let music_defs = model.function_definitions(&Name::from("music"));
    let itunes_defs = model.function_definitions(&Name::from("itunes"));
    assert_eq!(music_defs.len(), 1);
    assert_eq!(itunes_defs.len(), 1);
    assert_eq!(model.binding(music_defs[0]).span.slice(source), "music");
    assert_eq!(model.binding(itunes_defs[0]).span.slice(source), "itunes");

    let local_binding = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "track"
                && matches!(
                    binding.kind,
                    BindingKind::Declaration(DeclarationBuiltin::Local)
                )
        })
        .unwrap();
    let ScopeKind::Function(function_scope) = model.scope_kind(local_binding.scope) else {
        panic!("expected local binding to live in a function scope");
    };
    assert!(function_scope.contains_name_str("music"));
    assert!(function_scope.contains_name_str("itunes"));
    assert_eq!(function_scope.static_names().len(), 2);
}

#[test]
fn function_definition_binding_lookup_uses_the_command_span() {
    let source = "build() { :; }\nbuild() { echo later; }\n";
    let model = model(source);
    let name = Name::from("build");
    let definitions = model.function_definitions(&name);
    assert_eq!(definitions.len(), 2);

    let function_commands = model
        .commands()
        .iter()
        .copied()
        .filter(|command| model.command_kind(*command) == CommandKind::Function)
        .collect::<Vec<_>>();
    assert_eq!(function_commands.len(), 2);

    for (command, definition) in function_commands
        .into_iter()
        .zip(definitions.iter().copied())
    {
        assert_eq!(
            model.function_definition_binding_for_command_span(model.command_span(command)),
            Some(definition)
        );
    }
}

#[test]
fn binding_lookup_uses_the_definition_span() {
    let source = "foo=1\nbar=2\n";
    let model = model(source);

    for name in [Name::from("foo"), Name::from("bar")] {
        let binding_id = model.bindings_for(&name)[0];
        assert_eq!(
            model.binding_for_definition_span(model.binding(binding_id).span),
            Some(binding_id)
        );
    }
}

#[test]
fn zsh_multi_name_function_lookup_works_through_any_alias() {
    let source = "flag=1\nfunction music itunes() { echo \"$flag\"; }\nitunes\n";
    let model = model_with_dialect(source, ShellDialect::Zsh);

    assert_eq!(model.call_sites_for(&Name::from("itunes")).len(), 1);
    assert!(model.call_graph().reachable.contains(&Name::from("itunes")));
    assert!(
        !reportable_unused_names(&model)
            .into_iter()
            .any(|name| name == "flag")
    );
}

#[test]
fn semantic_analysis_exposes_function_scope_and_call_arity_bindings() {
    let source = "greet ok\ngreet() { echo \"$1\"; }\ngreet\n";
    let model = model(source);
    let analysis = model.analysis();
    let name = Name::from("greet");
    let binding = model.function_definitions(&name)[0];

    let function_scope = analysis
        .function_scope_for_binding(binding)
        .expect("expected function body scope");
    assert!(matches!(
        model.scope_kind(function_scope),
        ScopeKind::Function(_)
    ));

    let arity_sites = analysis
        .function_call_arity_sites(&name)
        .map(|(site, binding_id)| {
            (
                site.name_span.slice(source).to_owned(),
                site.arg_count,
                binding_id,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        arity_sites,
        vec![
            ("greet".to_owned(), 1, binding),
            ("greet".to_owned(), 0, binding)
        ]
    );
}

#[test]
fn semantic_analysis_exposes_case_cli_dispatch_reachability() {
    let source = "\
#!/bin/sh
start() { echo hi; }
case \"$1\" in
  start) \"$1\" ;;
esac
exit $?
late() { echo later; }
";
    let output = Parser::with_dialect(source, ShellDialect::Bash)
        .parse()
        .unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build(&output.file, source, &indexer);
    let analysis = model.analysis();
    let start_binding = model.function_definitions(&Name::from("start"))[0];
    let start_scope = analysis
        .function_scope_for_binding(start_binding)
        .expect("expected start function scope");
    let late_binding = model.function_definitions(&Name::from("late"))[0];
    let late_scope = analysis
        .function_scope_for_binding(late_binding)
        .expect("expected late function scope");

    let dispatches = analysis.case_cli_dispatches(&output.file, source);
    assert_eq!(dispatches.len(), 1);
    assert_eq!(dispatches[0].function_scope(), start_scope);
    assert_eq!(dispatches[0].dispatcher_span().slice(source), "\"$1\"");

    let reachable = analysis.case_cli_reachable_function_scopes(&output.file, &dispatches);
    assert!(reachable.contains(&start_scope));
    assert!(!reachable.contains(&late_scope));
}

#[test]
fn zsh_parameter_modifiers_still_register_references() {
    let model = model_with_profile("print ${(m)foo}\n", ShellProfile::native(ShellDialect::Zsh));
    let unresolved = unresolved_names(&model);

    assert_names_present(&["foo"], &unresolved);
}

#[test]
fn bash_profile_ignores_zsh_parameter_modifier_references() {
    let model = model_with_dialect("printf '%s\\n' ${=zsh_only} ${plain}\n", ShellDialect::Bash);
    let unresolved = unresolved_names(&model);

    assert_names_present(&["plain"], &unresolved);
    assert_names_absent(&["zsh_only"], &unresolved);
}

#[test]
fn zsh_parameter_operations_walk_operand_references_conservatively() {
    let model = model_with_profile(
        "print ${(m)foo#${needle}} ${(S)foo/$pattern/$replacement} ${(m)foo:$offset:${length}}\n",
        ShellProfile::native(ShellDialect::Zsh),
    );
    let unresolved = unresolved_names(&model);

    assert_names_present(
        &[
            "foo",
            "needle",
            "pattern",
            "replacement",
            "offset",
            "length",
        ],
        &unresolved,
    );
}

#[test]
fn zsh_for_loops_bind_all_targets() {
    let source = "\
for key value in a b c d; do
  print -r -- \"$key:$value\"
done
for 1 2 3; do
  print -r -- \"$1|$2|$3\"
done
";
    let model = model_with_dialect(source, ShellDialect::Zsh);

    let loop_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.kind == BindingKind::LoopVariable)
        .map(|binding| {
            (
                binding.name.to_string(),
                binding.span.slice(source).to_string(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        loop_bindings,
        vec![
            ("key".to_owned(), "key".to_owned()),
            ("value".to_owned(), "value".to_owned()),
            ("1".to_owned(), "1".to_owned()),
            ("2".to_owned(), "2".to_owned()),
            ("3".to_owned(), "3".to_owned()),
        ]
    );

    for name in ["key", "value", "1", "2", "3"] {
        let reference = model
            .references()
            .iter()
            .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == name)
            .unwrap_or_else(|| panic!("expected expansion reference for {name}"));
        let binding = model
            .resolved_binding(reference.id)
            .unwrap_or_else(|| panic!("expected {name} to resolve to a loop binding"));
        assert_eq!(binding.kind, BindingKind::LoopVariable);
        assert_eq!(binding.name, name);
    }
}

#[test]
fn classifies_assignment_and_function_binding_origins() {
    let source = "\
literal=plain
copy=$literal
fallback=${literal:-alt}
lower=${literal,}
quoted=${literal@Q}
name=literal
indirect=${!name}
build() { :; }
";
    let model = model(source);

    assert!(matches!(
        binding_for_name(&model, "literal").origin,
        BindingOrigin::Assignment {
            value: AssignmentValueOrigin::StaticLiteral,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "copy").origin,
        BindingOrigin::Assignment {
            value: AssignmentValueOrigin::PlainScalarAccess,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "fallback").origin,
        BindingOrigin::Assignment {
            value: AssignmentValueOrigin::ParameterOperator,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "lower").origin,
        BindingOrigin::Assignment {
            value: AssignmentValueOrigin::ParameterOperator,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "quoted").origin,
        BindingOrigin::Assignment {
            value: AssignmentValueOrigin::Transformation,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "indirect").origin,
        BindingOrigin::Assignment {
            value: AssignmentValueOrigin::IndirectExpansion,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "build").origin,
        BindingOrigin::FunctionDefinition { .. }
    ));
}

#[test]
fn assignment_definition_span_keeps_quoted_associative_subscript_text() {
    let source = "\
declare -A a
a[\"]=x\"]=1
";
    let model = model(source);
    let binding = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "a" && binding.kind == BindingKind::ArrayAssignment)
        .expect("expected subscripted assignment binding");

    let BindingOrigin::Assignment {
        definition_span, ..
    } = binding.origin
    else {
        panic!("expected assignment origin");
    };
    assert_eq!(definition_span.slice(source), "a[\"]=x\"]");
}

#[test]
fn classifies_loop_and_parameter_default_origins() {
    let source = "\
for size in 16 32; do
  :
done
name=world
for item in $name; do
  :
done
for arg; do
  :
done
: \"${created:=default}\"
";
    let model = model(source);

    assert!(matches!(
        binding_for_name(&model, "size").origin,
        BindingOrigin::LoopVariable {
            items: LoopValueOrigin::StaticWords,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "item").origin,
        BindingOrigin::LoopVariable {
            items: LoopValueOrigin::ExpandedWords,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "arg").origin,
        BindingOrigin::LoopVariable {
            items: LoopValueOrigin::ImplicitArgv,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "created").origin,
        BindingOrigin::ParameterDefaultAssignment { .. }
    ));
}

#[test]
fn classifies_builtin_target_and_imported_origins() {
    let source = "\
read reply
mapfile lines
printf -v rendered '%s' hi
while getopts 'a' opt; do
  :
done
printf '%s\\n' \"$pkgname\"
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build_with_options(
        &output.file,
        source,
        &indexer,
        SemanticBuildOptions {
            file_entry_contract: Some(FileContract {
                required_reads: Vec::new(),
                provided_bindings: vec![ProvidedBinding::new(
                    Name::from("pkgname"),
                    ProvidedBindingKind::Variable,
                    ContractCertainty::Definite,
                )],
                provided_functions: Vec::new(),
                externally_consumed_bindings: false,
            }),
            ..SemanticBuildOptions::default()
        },
    );

    assert!(matches!(
        binding_for_name(&model, "reply").origin,
        BindingOrigin::BuiltinTarget {
            kind: BuiltinBindingTargetKind::Read,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "lines").origin,
        BindingOrigin::BuiltinTarget {
            kind: BuiltinBindingTargetKind::Mapfile,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "rendered").origin,
        BindingOrigin::BuiltinTarget {
            kind: BuiltinBindingTargetKind::Printf,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "opt").origin,
        BindingOrigin::BuiltinTarget {
            kind: BuiltinBindingTargetKind::Getopts,
            ..
        }
    ));
    assert!(matches!(
        binding_for_name(&model, "pkgname").origin,
        BindingOrigin::Imported { .. }
    ));
}

#[test]
fn escaped_declarations_create_name_bindings() {
    let source = "\\typeset result temp_flags\nprintf '%s\\n' \"$result\"\n";
    let model = model(source);
    let unused = model.analysis().unused_assignments().to_vec();

    assert!(!binding_for_name(&model, "result").references.is_empty());
    assert_eq!(binding_names(&model, &unused), vec!["temp_flags"]);
    assert!(matches!(
        binding_for_name(&model, "temp_flags").kind,
        BindingKind::Declaration(DeclarationBuiltin::Typeset)
    ));
}

#[test]
fn escaped_exported_declarations_keep_exported_attributes() {
    let source = "\\typeset -x rvm_hook\nrvm_hook=after_update\n";
    let model = model(source);
    let bindings = model.bindings_for(&Name::from("rvm_hook"));

    assert!(
        bindings.iter().any(|binding_id| {
            model
                .binding(*binding_id)
                .attributes
                .contains(BindingAttributes::EXPORTED)
        }),
        "expected an exported rvm_hook declaration"
    );
}

#[test]
fn wrapped_read_creates_read_target_bindings() {
    let source = "builtin read -${flag} 1 -s -r anykey\n";
    let model = model(source);
    let anykey = binding_for_name(&model, "anykey");

    assert!(matches!(anykey.kind, BindingKind::ReadTarget));
    assert!(matches!(
        anykey.origin,
        BindingOrigin::BuiltinTarget {
            kind: BuiltinBindingTargetKind::Read,
            ..
        }
    ));
}

#[test]
fn command_wrapped_printf_v_creates_target_binding() {
    let source = "command printf -v rendered '%s' value\n";
    let model = model(source);
    let rendered = binding_for_name(&model, "rendered");

    assert!(matches!(rendered.kind, BindingKind::PrintfTarget));
    assert!(matches!(
        rendered.origin,
        BindingOrigin::BuiltinTarget {
            kind: BuiltinBindingTargetKind::Printf,
            ..
        }
    ));
}

#[test]
fn shflags_define_commands_create_flag_bindings() {
    let source = "DEFINE_boolean show_commands false 'show commands' s\n";
    let model = model(source);
    let binding = binding_for_name(&model, "FLAGS_show_commands");

    assert_eq!(binding.span.slice(source), "show_commands");
    assert!(matches!(
        binding.kind,
        BindingKind::Declaration(DeclarationBuiltin::Declare)
    ));
}

#[test]
fn isolates_subshell_bindings_from_parent_resolution() {
    let source = "VAR=outer\n( VAR=inner )\necho $VAR\n";
    let model = model(source);

    let reference = model.references().last().unwrap();
    let binding = model.resolved_binding(reference.id).unwrap();
    assert_eq!(binding.span.slice(source), "VAR");
}

#[test]
fn declare_g_in_command_substitution_stays_in_that_execution_scope() {
    let source = "\
#!/bin/bash
printf '%s\\n' \"$(
  f() { declare -gA assoc=([key]=1); }
  f
  printf '%s\\n' \"${assoc[key]}\"
)\"
printf '%s\\n' \"${assoc[key]}\"
";
    let model = model(source);

    let assoc_binding = binding_for_name(&model, "assoc");
    assert!(assoc_binding.attributes.contains(BindingAttributes::ARRAY));
    assert!(assoc_binding.attributes.contains(BindingAttributes::ASSOC));
    assert!(matches!(
        model.scope_kind(assoc_binding.scope),
        ScopeKind::CommandSubstitution
    ));
}

#[test]
fn records_pipeline_segment_scopes() {
    let source = "a | b | c\n";
    let model = model(source);

    let pipeline_scopes = model
        .scopes()
        .iter()
        .filter(|scope| matches!(scope.kind, ScopeKind::Pipeline))
        .count();
    assert_eq!(pipeline_scopes, 3);
}

#[test]
fn indexed_scope_lookup_matches_linear_scan_for_all_offsets() {
    let source = "\
outer() {
  local current=1
  (
    printf '%s\\n' \"$(
      printf '%s\\n' \"$current\" | tr a b
    )\"
  )
  inner() { echo \"$current\"; }
}
outer
";
    let model = model(source);

    for offset in 0..=source.len() {
        assert_eq!(
            model.scope_at(offset),
            linear_scope_at(model.scopes(), offset),
            "offset {offset}"
        );
    }
}

#[test]
fn recorded_program_preserves_logical_list_order_in_ranges() {
    let source = "a && b || c\n";
    let model = model(source);

    let lists = model.list_commands();
    assert_eq!(lists.len(), 1);
    let segments = &lists[0].segments;

    assert_eq!(segments.len(), 3);
    assert!(segments[0].command_span.slice(source).starts_with("a"));
    assert!(segments[1].command_span.slice(source).starts_with("b"));
    assert!(segments[2].command_span.slice(source).starts_with("c"));
}

#[test]
fn flattened_logical_lists_preserve_short_circuit_cfg_paths() {
    let source = "true && true || exit 1\nprintf '%s\\n' reachable\n";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn recorded_program_preserves_pipeline_segment_order_in_ranges() {
    let source = "a | b | c\n";
    let model = model(source);

    let pipelines = model.pipeline_commands();
    assert_eq!(pipelines.len(), 1);
    let segments = &pipelines[0].segments;

    assert_eq!(segments.len(), 3);
    assert!(segments[0].command_span.slice(source).starts_with("a"));
    assert!(segments[1].command_span.slice(source).starts_with("b"));
    assert!(segments[2].command_span.slice(source).starts_with("c"));
}

#[test]
fn command_topology_exposes_parent_child_relationships() {
    let source = "if foo; then bar | baz; fi\n";
    let model = model(source);

    let if_id = command_id_starting_with(&model, source, "if foo").unwrap();
    let foo_id = command_id_starting_with(&model, source, "foo").unwrap();
    let pipeline_id = command_id_containing(&model, source, "bar | baz").unwrap();
    let bar_id = command_id_starting_with(&model, source, "bar").unwrap();
    let baz_id = command_id_starting_with(&model, source, "baz").unwrap();

    assert_eq!(model.command_parent_id(foo_id), Some(if_id));
    assert_eq!(model.command_parent_id(pipeline_id), Some(if_id));
    assert_eq!(model.command_parent_id(bar_id), Some(pipeline_id));
    assert_eq!(model.command_parent_id(baz_id), Some(pipeline_id));
    assert!(model.command_children(if_id).contains(&foo_id));
    assert!(model.command_children(if_id).contains(&pipeline_id));
}

#[test]
fn command_topology_attaches_nested_function_body_to_nearest_function() {
    let source = "\
outer() {
  inner() {
    echo inner
  }
}
";
    let model = model(source);

    let inner_function_id = command_id_starting_with(&model, source, "inner()").unwrap();
    let inner_body_id = model
        .commands()
        .iter()
        .copied()
        .filter(|id| {
            model.command_kind(*id) == CommandKind::Compound(CompoundCommandKind::BraceGroup)
                && model
                    .command_syntax_span(*id)
                    .slice(source)
                    .contains("echo inner")
        })
        .min_by_key(|id| {
            let span = model.command_syntax_span(*id);
            span.end.offset - span.start.offset
        })
        .unwrap();

    assert_eq!(
        model.command_parent_id(inner_body_id),
        Some(inner_function_id)
    );
    assert!(
        model
            .command_children(inner_function_id)
            .contains(&inner_body_id)
    );
}

#[test]
fn command_topology_excludes_nested_substitutions_from_structural_commands() {
    let source = "echo \"$(printf '%s\\n' hi)\"\n";
    let model = model(source);

    let echo_id = command_id_starting_with(&model, source, "echo").unwrap();
    let printf_id = command_id_starting_with(&model, source, "printf").unwrap();

    assert_eq!(model.command_parent_id(printf_id), Some(echo_id));
    assert!(model.structural_commands().contains(&echo_id));
    assert!(!model.structural_commands().contains(&printf_id));
}

#[test]
fn command_topology_skips_synthetic_parents_for_syntax_backed_queries() {
    let source = "case \"$x\" in $(echo \"$v\")) ;; esac\n";
    let model = model(source);

    let case_id = command_id_starting_with(&model, source, "case").unwrap();
    let echo_id = command_id_starting_with(&model, source, "echo").unwrap();

    assert_eq!(
        model.syntax_backed_command_parent_id(echo_id),
        Some(case_id)
    );
    assert!(
        model
            .syntax_backed_command_children(case_id)
            .contains(&echo_id)
    );
}

#[test]
fn command_lookup_by_span_and_kind_skips_synthetic_commands() {
    let source = "case \"$x\" in $(echo a)) ;; esac\n";
    let model = model(source);

    let case_span = model
        .commands()
        .iter()
        .copied()
        .map(|id| model.command_syntax_span(id))
        .find(|span| span.slice(source).starts_with("case"))
        .unwrap();

    assert!(
        model
            .command_by_span_and_kind(case_span, CommandKind::Compound(CompoundCommandKind::Case))
            .is_some()
    );
    let case_id = model.command_by_span(case_span).unwrap();
    assert_eq!(
        model.command_kind(case_id),
        CommandKind::Compound(CompoundCommandKind::Case)
    );
    let innermost_case_id = model.innermost_command_id_at(source.find("case").unwrap());
    assert_eq!(innermost_case_id, Some(case_id));
}

#[test]
fn commands_iter_filters_synthetic_ids_so_command_kind_is_safe() {
    let source = "case \"$x\" in $(echo a)) ;; esac\n";
    let model = model(source);

    for id in model.commands().iter().copied() {
        let _ = model.command_kind(id);
    }
}

#[test]
fn command_topology_preserves_nested_region_immediate_parents() {
    let source = "echo >\"$(a | b)\"\n";
    let model = model(source);

    let echo_id = command_id_starting_with(&model, source, "echo").unwrap();
    let pipeline_span = model.pipeline_commands()[0].span;
    let pipeline_id = model
        .command_by_span_and_kind(pipeline_span, CommandKind::Binary)
        .unwrap();
    let a_id = command_id_starting_with(&model, source, "a").unwrap();
    let b_id = command_id_starting_with(&model, source, "b").unwrap();

    assert_eq!(model.command_parent_id(pipeline_id), Some(echo_id));
    assert_eq!(model.command_parent_id(a_id), Some(pipeline_id));
    assert_eq!(model.command_parent_id(b_id), Some(pipeline_id));
}

#[test]
fn command_topology_finds_innermost_command_at_offsets() {
    let source = "if foo; then bar; fi\n";
    let model = model(source);

    let if_id = command_id_starting_with(&model, source, "if foo").unwrap();
    let foo_id = command_id_starting_with(&model, source, "foo").unwrap();
    let bar_id = command_id_starting_with(&model, source, "bar").unwrap();

    assert_eq!(
        model.innermost_command_id_at(source.find("if").unwrap()),
        Some(if_id)
    );
    assert_eq!(
        model.innermost_command_id_at(source.find("foo").unwrap()),
        Some(foo_id)
    );
    assert_eq!(
        model.innermost_command_id_at(source.find("bar").unwrap()),
        Some(bar_id)
    );
}

#[test]
fn command_topology_finds_innermost_command_containing_arbitrary_offsets() {
    let source = "if foo; then bar; fi\n";
    let model = model(source);

    let if_id = command_id_starting_with(&model, source, "if foo").unwrap();
    let foo_id = command_id_starting_with(&model, source, "foo").unwrap();
    let bar_id = command_id_starting_with(&model, source, "bar").unwrap();

    assert_eq!(
        model.innermost_command_id_containing_offset(source.find("if").unwrap() + 1),
        Some(if_id)
    );
    assert_eq!(
        model.innermost_command_id_containing_offset(source.find("foo").unwrap() + 1),
        Some(foo_id)
    );
    assert_eq!(
        model.innermost_command_id_containing_offset(source.find("bar").unwrap() + 1),
        Some(bar_id)
    );
}

#[test]
fn arithmetic_plain_assignment_is_write_only() {
    let model = model("(( i = 0 ))\n");
    assert_arithmetic_usage(&model, "i", 0, 1);
}

#[test]
fn arithmetic_compound_assignment_is_read_write() {
    let model = model("(( i += 2 ))\n");
    assert_arithmetic_usage(&model, "i", 1, 1);
}

#[test]
fn arithmetic_prefix_update_is_read_write() {
    let model = model("(( ++i ))\n");
    assert_arithmetic_usage(&model, "i", 1, 1);
}

#[test]
fn arithmetic_postfix_update_is_read_write() {
    let model = model("(( i++ ))\n");
    assert_arithmetic_usage(&model, "i", 1, 1);
}

#[test]
fn arithmetic_assignment_reads_index_expressions() {
    let model = model("(( a[i++] = 1 ))\n");
    assert_arithmetic_usage(&model, "a", 0, 1);
    assert_arithmetic_usage(&model, "i", 1, 1);
}

#[test]
fn arithmetic_conditional_tracks_branch_reads_and_writes() {
    let model = model("(( x ? y++ : (z = 1) ))\n");
    assert_arithmetic_usage(&model, "x", 1, 0);
    assert_arithmetic_usage(&model, "y", 1, 1);
    assert_arithmetic_usage(&model, "z", 0, 1);
}

#[test]
fn arithmetic_comma_walks_each_expression_in_order() {
    let model = model("(( x = 1, y += x, z ))\n");
    assert_arithmetic_usage(&model, "x", 1, 1);
    assert_arithmetic_usage(&model, "y", 1, 1);
    assert_arithmetic_usage(&model, "z", 1, 0);
}

#[test]
fn arithmetic_shell_words_still_walk_nested_expansions() {
    let model = model("echo $(( $(printf '%s' \"$x\") + y ))\n");
    assert!(
        model.references().iter().any(|reference| {
            reference.kind == ReferenceKind::Expansion && reference.name == "x"
        })
    );
    assert_arithmetic_usage(&model, "y", 1, 0);
}

#[test]
fn substring_offset_arithmetic_tracks_postfix_updates() {
    let source = "\
#!/bin/bash
spinner() {
  local chars=\"/-\\\\|\"
  local spin_i=0
  while true; do
    printf '%s\\n' \"${chars:spin_i++%${#chars}:1}\"
  done
}
";
    let model = model(source);
    assert!(model.references().iter().any(|reference| {
        reference.kind == ReferenceKind::ParameterSliceArithmetic && reference.name == "spin_i"
    }));
    assert_eq!(
        arithmetic_write_count(&model, "spin_i"),
        1,
        "unexpected arithmetic write count for spin_i"
    );

    let unused = model
        .analysis()
        .unused_assignments()
        .iter()
        .map(|binding| model.binding(*binding).name.as_str())
        .collect::<Vec<_>>();
    assert!(!unused.contains(&"spin_i"), "unused bindings: {:?}", unused);
}

#[test]
fn classifies_nameref_and_source_directives() {
    let source = "\
declare -n ref=target
# shellcheck source=lib.sh
source \"$x\"
";
    let model = model(source);

    let nameref = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "ref")
        .unwrap();
    assert!(matches!(nameref.kind, BindingKind::Nameref));
    assert!(nameref.attributes.contains(BindingAttributes::NAMEREF));

    assert_eq!(
        model.source_refs()[0].kind,
        SourceRefKind::Directive("lib.sh".into())
    );
    assert_eq!(
        model.source_refs()[0].resolution,
        SourceRefResolution::Unchecked
    );
}

#[test]
fn source_directive_applies_across_contiguous_own_line_comments() {
    let source = "\
# shellcheck source=lib.sh
# shellcheck disable=SC2154
source \"$x\"
";
    let model = model(source);

    assert_eq!(
        model.source_refs()[0].kind,
        SourceRefKind::Directive("lib.sh".into())
    );
}

#[test]
fn dev_null_source_directive_persists_until_overridden() {
    let source = "\
# shellcheck source=/dev/null
foo() { echo hi; }
source \"$a\"
source \"$b\"
# shellcheck source=./helper.sh
source \"$c\"
source \"$d\"
";
    let model = model(source);

    assert_eq!(model.source_refs().len(), 4);
    assert_eq!(model.source_refs()[0].kind, SourceRefKind::DirectiveDevNull);
    assert_eq!(model.source_refs()[1].kind, SourceRefKind::DirectiveDevNull);
    assert_eq!(
        model.source_refs()[2].kind,
        SourceRefKind::Directive("./helper.sh".into())
    );
    assert_eq!(model.source_refs()[3].kind, SourceRefKind::Dynamic);
}

#[test]
fn escaped_dot_source_builtin_still_records_dynamic_source_refs() {
    let source = "\
#!/bin/bash
\\. \"$rvm_environments_path/$1\"
";
    let model = model(source);

    assert_eq!(model.source_refs().len(), 1);
    assert_eq!(model.source_refs()[0].kind, SourceRefKind::Dynamic);
    assert_eq!(
        model.source_refs()[0].diagnostic_class,
        SourceRefDiagnosticClass::DynamicPath
    );
}

#[test]
fn command_wrappers_do_not_create_source_refs() {
    let source = "\
#!/bin/bash
command . \"$file\"
builtin source \"$file\"
noglob . \"$file\"
";
    let model = model(source);

    assert!(model.source_refs().is_empty());
}

#[test]
fn parameter_expansion_roots_with_static_path_tails_are_untracked_source_refs() {
    for source in [
        "#!/bin/bash\nsource \"${ROOT?}/helper.sh\"\n",
        "#!/bin/bash\nsource \"${ROOT:-$HOME/.config}/helper.sh\"\n",
        "#!/bin/bash\nsource \"${ROOT+vendor}/helper.sh\"\n",
        "#!/bin/bash\nsource \"${#ROOT}/helper.sh\"\n",
        "#!/bin/bash\nsource \"${ROOT%/*}/helper.sh\"\n",
        "#!/bin/bash\nsource \"${ROOT/file/repl}/helper.sh\"\n",
        "#!/bin/bash\nsource \"${!ROOT}/helper.sh\"\n",
    ] {
        let model = model(source);

        assert_eq!(model.source_refs().len(), 1, "{source}");
        assert_eq!(
            model.source_refs()[0].kind,
            SourceRefKind::Dynamic,
            "{source}"
        );
        assert_eq!(
            model.source_refs()[0].diagnostic_class,
            SourceRefDiagnosticClass::UntrackedFile,
            "{source}"
        );
    }
}

#[test]
fn command_substitution_roots_with_static_path_tails_are_untracked_source_refs() {
    for source in [
        "#!/bin/bash\nsource \"$(git --exec-path)/git-sh-setup\"\n",
        "#!/bin/sh\n. \"$(dirname \"$0\")/autopause-fcns.sh\"\n",
        "#!/bin/ksh\nsource \"$(cd \"$(dirname \"${0}\")\"; pwd)/../nb\"\n",
    ] {
        let model = model(source);

        assert_eq!(model.source_refs().len(), 1, "{source}");
        assert_eq!(
            model.source_refs()[0].kind,
            SourceRefKind::Dynamic,
            "{source}"
        );
        assert_eq!(
            model.source_refs()[0].diagnostic_class,
            SourceRefDiagnosticClass::UntrackedFile,
            "{source}"
        );
    }
}

#[test]
fn literal_leading_backslashes_do_not_create_source_refs() {
    for source in [
        "#!/bin/bash\n\"\\\\.\" \"$rvm_environments_path/$1\"\n",
        "#!/bin/bash\n'\\source' \"$rvm_environments_path/$1\"\n",
        "#!/bin/bash\n\\\\. \"$rvm_environments_path/$1\"\n",
    ] {
        let model = model(source);

        assert!(
            model.source_refs().is_empty(),
            "unexpected source refs for {source:?}: {:?}",
            model.source_refs()
        );
    }
}

#[test]
fn builds_transitive_call_graph_and_overwritten_functions() {
    let source = "\
f() { g; }
g() { echo hi; }
f
f() { echo again; }
";
    let model = model(source);

    assert!(model.call_graph().reachable.contains("f"));
    assert!(model.call_graph().reachable.contains("g"));
    assert_eq!(model.call_graph().overwritten.len(), 1);
    assert_eq!(model.call_graph().overwritten[0].name, "f");
}

#[test]
fn call_graph_does_not_root_calls_inside_uncalled_functions() {
    let source = "\
f() { g; }
g() { echo hi; }
";
    let model = model(source);

    assert!(!model.call_graph().reachable.contains("f"));
    assert!(!model.call_graph().reachable.contains("g"));
}

#[test]
fn precise_overwritten_functions_track_real_overwrites() {
    let source = "\
f() { echo hi; }
f() { echo again; }
";
    let model = model(source);
    let analysis = model.analysis();
    let overwritten = analysis.overwritten_functions();

    assert_eq!(overwritten.len(), 1);
    assert_eq!(overwritten[0].name, "f");
    assert!(!overwritten[0].first_called);
}

#[test]
fn precise_overwritten_functions_preserve_calls_before_redefinition() {
    let source = "\
f() { echo hi; }
f
f() { echo again; }
";
    let model = model(source);
    let analysis = model.analysis();
    let overwritten = analysis.overwritten_functions();

    assert_eq!(overwritten.len(), 1);
    assert!(overwritten[0].first_called);
}

#[test]
fn precise_overwritten_functions_count_nested_calls_from_invoked_wrappers() {
    let source = "\
run_case() {
  helper
}
helper() { echo hi; }
run_case
helper() { echo again; }
";
    let model = model(source);
    let analysis = model.analysis();
    let overwritten = analysis.overwritten_functions();

    assert_eq!(overwritten.len(), 1);
    assert!(overwritten[0].first_called);
}

#[test]
fn precise_overwritten_functions_do_not_count_shadowed_nested_calls() {
    let source = "\
run_case() {
  helper() { echo local; }
  helper
}
helper() { echo hi; }
run_case
helper() { echo again; }
";
    let model = model(source);
    let analysis = model.analysis();
    let overwritten = analysis.overwritten_functions();

    assert_eq!(overwritten.len(), 1);
    assert!(!overwritten[0].first_called);
}

#[test]
fn precise_overwritten_functions_ignore_mutually_exclusive_branches() {
    let source = "\
if cond; then
  helper() { return 0; }
else
  helper() { return 1; }
fi
helper
";
    let model = model(source);

    assert!(model.analysis().overwritten_functions().is_empty());
}

#[test]
fn precise_overwritten_functions_ignore_conditional_redefinition_after_default() {
    let source = "\
helper() { return 0; }
if cond; then
  helper() { return 1; }
fi
helper
";
    let model = model(source);

    assert!(model.analysis().overwritten_functions().is_empty());
}

#[test]
fn precise_overwritten_functions_allow_terminating_paths_before_redefinition() {
    let source = "\
helper() { return 0; }
maybe || exit 1
helper() { return 1; }
";
    let model = model(source);
    let analysis = model.analysis();
    let overwritten = analysis.overwritten_functions();

    assert_eq!(overwritten.len(), 1);
    assert_eq!(overwritten[0].name, "helper");
    assert!(!overwritten[0].first_called);
}

#[test]
fn precise_overwritten_functions_do_not_merge_distinct_helper_scopes() {
    let source = "\
factory_one() {
  helper() { return 0; }
  helper
}
factory_two() {
  helper() { return 1; }
  helper
}
factory_one
factory_two
";
    let model = model(source);

    assert!(model.analysis().overwritten_functions().is_empty());
}

#[test]
fn unreached_functions_report_definitions_before_script_termination() {
    let source = "f() { echo hi; }\nexit 0\n";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions();

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "f");
    assert_eq!(
        unreached[0].reason,
        UnreachedFunctionReason::ScriptTerminates
    );
}

#[test]
fn unreached_functions_report_definitions_before_terminating_function_call() {
    let source = "\
f() { echo hi; }
die() { exit 0; }
die
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions();

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "f");
    assert_eq!(
        unreached[0].reason,
        UnreachedFunctionReason::ScriptTerminates
    );
}

#[test]
fn unreached_functions_ignore_definitions_at_plain_eof() {
    let source = "f() { echo hi; }\n";
    let model = model(source);

    assert!(model.analysis().unreached_functions().is_empty());
}

#[test]
fn unreached_functions_report_nested_plain_eof_only_with_compat_option() {
    let source = "\
outer() {
  inner() { :; }
}
";
    let model = model(source);
    let analysis = model.analysis();

    assert!(analysis.unreached_functions().is_empty());

    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "inner");
    assert_eq!(
        unreached[0].reason,
        UnreachedFunctionReason::EnclosingFunctionUnreached
    );
}

#[test]
fn unreached_functions_report_subshell_definitions_with_compat_option() {
    let source = "( inner() { :; }; )\n";
    let model = model(source);
    let analysis = model.analysis();

    assert!(analysis.unreached_functions().is_empty());

    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "inner");
    assert_eq!(
        unreached[0].reason,
        UnreachedFunctionReason::EnclosingFunctionUnreached
    );
}

#[test]
fn unreached_functions_ignore_nested_definition_when_enclosing_function_is_called() {
    let source = "\
outer() {
  inner() { :; }
}
outer
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert!(unreached.is_empty());
}

#[test]
fn unreached_functions_do_not_count_indirect_enclosing_function_calls() {
    let source = "\
outer() {
  inner() { :; }
}
name=outer
$name
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "inner");
}

#[test]
fn unreached_functions_count_same_scope_nested_calls_before_enclosing_scope_exits() {
    let source = "\
outer() {
  inner() { :; }
  inner
}
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert!(unreached.is_empty());
}

#[test]
fn unreached_functions_count_same_scope_command_substitution_calls() {
    let source = "\
outer() {
  inner() { :; }
  value=$(inner)
}
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert!(unreached.is_empty());
}

#[test]
fn unreached_functions_ignore_inner_shadow_inside_command_substitution() {
    let source = "\
outer() {
  inner() { :; }
  value=$(inner() { :; }; inner)
}
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "inner");
}

#[test]
fn transient_call_sites_include_function_scopes_under_command_substitution() {
    let source = "\
printf '%s\\n' \"$(
  caller() {
    target
  }
  caller
)\"
";
    let model = model(source);
    let analysis = model.analysis();
    let site = &model.call_sites_for(&Name::from("target"))[0];

    assert!(analysis.call_site_runs_in_transient_context(site.scope));
}

#[test]
fn function_scope_boundary_helpers_ignore_outer_transient_ancestors() {
    let source = "\
printf '%s\\n' \"$(
  caller() {
    printf '%s\\n' \"$1\"
  }
  caller
)\"
";
    let model = model(source);
    let scope = model.scope_at(
        span_for_nth(source, "printf '%s\\n' \"$1\"", 0)
            .start
            .offset,
    );

    assert!(
        model
            .transient_ancestor_scopes_within_function(scope)
            .next()
            .is_none()
    );
    assert!(
        model
            .innermost_transient_scope_within_function(scope)
            .is_none()
    );
    assert_eq!(
        model.enclosing_function_scope_without_transient_boundary(scope),
        model.enclosing_function_scope(scope)
    );
}

#[test]
fn function_scope_boundary_helpers_stop_at_in_function_transient_scopes() {
    let source = "\
caller() {
  value=\"$(
    printf '%s\\n' \"$1\"
  )\"
}
";
    let model = model(source);
    let scope = model.scope_at(
        span_for_nth(source, "printf '%s\\n' \"$1\"", 0)
            .start
            .offset,
    );
    let transients = model
        .transient_ancestor_scopes_within_function(scope)
        .collect::<Vec<_>>();

    assert_eq!(transients.len(), 1);
    assert!(matches!(
        model.scope_kind(transients[0]),
        ScopeKind::CommandSubstitution
    ));
    assert_eq!(
        model.innermost_transient_scope_within_function(scope),
        Some(transients[0])
    );
    assert_eq!(
        model.enclosing_function_scope_without_transient_boundary(scope),
        None
    );
}

#[test]
fn function_call_reachability_finds_direct_call_before_cutoff() {
    let source = "\
target() { :; }
target
exit
";
    let model = model(source);
    let binding = function_binding_id(&model, "target", 0);
    let cutoff = span_for_nth(source, "exit", 0).start.offset;
    let analysis = model.analysis();
    let mut reachability = analysis.direct_function_call_reachability(Vec::new());

    assert!(reachability.binding_has_reachable_direct_call(
        binding,
        DirectFunctionCallWindow::before_offset(cutoff)
    ));
}

#[test]
fn function_call_reachability_respects_between_offsets() {
    let source = "\
target() { :; }
echo before
target
exit
";
    let model = model(source);
    let binding = function_binding_id(&model, "target", 0);
    let after = span_for_nth(source, "echo", 0).start.offset;
    let before = span_for_nth(source, "exit", 0).start.offset;
    let early_before = span_for_nth(source, "target", 1).start.offset - 1;
    let analysis = model.analysis();
    let mut reachability = analysis.direct_function_call_reachability(Vec::new());

    assert!(reachability.binding_has_reachable_direct_call(
        binding,
        DirectFunctionCallWindow::between_offsets(after, before)
    ));
    assert!(!reachability.binding_has_reachable_direct_call(
        binding,
        DirectFunctionCallWindow::between_offsets(after, early_before)
    ));
}

#[test]
fn function_call_reachability_persistent_query_excludes_transient_calls() {
    let source = "\
target() { :; }
value=$(target)
";
    let model = model(source);
    let binding = function_binding_id(&model, "target", 0);
    let analysis = model.analysis();
    let mut reachability = analysis.direct_function_call_reachability(Vec::new());

    assert!(reachability.binding_has_reachable_direct_call(
        binding,
        DirectFunctionCallWindow::before_offset(source.len())
    ));
    assert!(!reachability.binding_has_reachable_direct_call(
        binding,
        DirectFunctionCallWindow::before_offset(source.len()).persistent()
    ));
}

#[test]
fn function_call_reachability_follows_nested_function_activation() {
    let source = "\
target() { :; }
wrapper() {
  target
}
wrapper
";
    let model = model(source);
    let binding = function_binding_id(&model, "target", 0);
    let analysis = model.analysis();
    let mut reachability = analysis.direct_function_call_reachability(Vec::new());

    assert!(reachability.binding_has_reachable_direct_call(
        binding,
        DirectFunctionCallWindow::before_offset(source.len())
    ));
}

#[test]
fn function_call_reachability_blocks_prior_shadowed_binding() {
    let source = "\
target() { : old; }
target() { : new; }
target
";
    let model = model(source);
    let first = function_binding_id(&model, "target", 0);
    let second = function_binding_id(&model, "target", 1);
    let analysis = model.analysis();
    let mut reachability = analysis.direct_function_call_reachability(Vec::new());

    assert!(!reachability.binding_has_reachable_direct_call(
        first,
        DirectFunctionCallWindow::before_offset(source.len())
    ));
    assert!(reachability.binding_has_reachable_direct_call(
        second,
        DirectFunctionCallWindow::before_offset(source.len())
    ));
}

#[test]
fn function_call_reachability_uses_supplemental_call_candidates() {
    let source = "\
target() { :; }
command target
";
    let model = model(source);
    let binding = function_binding_id(&model, "target", 0);
    let target_span = span_for_nth(source, "target", 1);
    let command_span = span_for_nth(source, "command target", 0);
    let supplemental = vec![FunctionCallCandidate {
        callee: Name::from("target"),
        scope: model.scope_at(target_span.start.offset),
        name_span: target_span,
        command_span,
    }];

    let analysis = model.analysis();
    let mut without_supplemental = analysis.direct_function_call_reachability(Vec::new());
    assert!(!without_supplemental.binding_has_reachable_direct_call(
        binding,
        DirectFunctionCallWindow::before_offset(source.len())
    ));

    let mut with_supplemental = analysis.direct_function_call_reachability(supplemental);
    assert!(with_supplemental.binding_has_reachable_direct_call(
        binding,
        DirectFunctionCallWindow::before_offset(source.len())
    ));
}

#[test]
fn unreached_functions_count_transitive_nested_calls_before_enclosing_scope_exits() {
    let source = "\
outer() {
  inner() { :; }
  wrapper() {
    inner
  }
  wrapper
}
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert!(unreached.is_empty());
}

#[test]
fn unreached_functions_follow_nested_direct_call_chains() {
    let source = "\
outer() {
  wrapper() {
    inner() { :; }
  }
  wrapper
}
outer
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert!(unreached.is_empty());
}

#[test]
fn unreached_functions_count_nested_definitions_installed_before_later_body_call() {
    let source = "\
provider() {
  helper() { :; }
}
consumer() {
  provider
  helper
}
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert!(unreached.is_empty());
}

#[test]
fn unreached_functions_report_nested_definitions_when_provider_call_follows_use() {
    let source = "\
provider() {
  helper() { :; }
}
consumer() {
  helper
  provider
}
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "helper");
}

#[test]
fn unreached_functions_do_not_count_provider_call_when_consumer_runs_before_definition() {
    let source = "\
consumer() {
  provider
  helper
}
consumer
provider() {
  helper() { :; }
}
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "helper");
}

#[test]
fn unreached_functions_ignore_unreachable_consumer_calls_before_provider_definition() {
    let source = "\
consumer() {
  provider
  helper
}
guard() {
  return
  consumer
}
guard
provider() {
  helper() { :; }
}
consumer
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert!(unreached.is_empty());
}

#[test]
fn unreached_functions_report_nested_definitions_when_intermediate_scope_is_not_called() {
    let source = "\
outer() {
  wrapper() {
    inner() { :; }
  }
}
outer
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "inner");
}

#[test]
fn unreached_functions_do_not_count_calls_before_enclosing_definition() {
    let source = "\
outer
outer() {
  inner() { :; }
}
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions_with_options(UnreachedFunctionAnalysisOptions {
        report_unreached_nested_definitions: true,
    });

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "inner");
}

#[test]
fn unreached_functions_ignore_direct_calls_before_termination() {
    let source = "f() { echo hi; }\nf\nexit 0\n";
    let model = model(source);

    assert!(model.analysis().unreached_functions().is_empty());
}

#[test]
fn unreached_functions_count_direct_calls_that_terminate() {
    let source = "f() { exit 1; }\nf\n";
    let model = model(source);

    assert!(model.analysis().unreached_functions().is_empty());
}

#[test]
fn unreached_functions_report_unreachable_definitions() {
    let source = "exit 0\nf() { echo hi; }\n";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions();

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "f");
    assert_eq!(
        unreached[0].reason,
        UnreachedFunctionReason::UnreachableDefinition
    );
}

#[test]
fn unreached_functions_ignore_non_guaranteed_termination() {
    let source = "f() { echo hi; }\nif cond; then exit 0; fi\n";
    let model = model(source);

    assert!(model.analysis().unreached_functions().is_empty());
}

#[test]
fn unreached_functions_count_transitive_direct_calls_before_termination() {
    let source = "\
run_case() {
  helper
}
helper() { echo hi; }
run_case
exit 0
";
    let model = model(source);

    assert!(model.analysis().unreached_functions().is_empty());
}

#[test]
fn unreached_functions_ignore_body_calls_when_enclosing_function_runs_before_definition() {
    let source = "\
helper() { :; }
main() {
  late
}
main
late() {
  helper
}
exit 0
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions();
    let names = unreached
        .iter()
        .map(|function| function.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, ["helper", "late"]);
}

#[test]
fn unreached_functions_count_nested_substitution_calls_before_termination() {
    let source = "\
run_case() {
  output=\"$(echo value | helper)\"
}
helper() { echo hi; }
run_case
exit 0
";
    let model = model(source);

    assert!(model.analysis().unreached_functions().is_empty());
}

#[test]
fn unreached_functions_ignore_shadowed_command_substitution_calls_before_termination() {
    let source = "\
helper() { echo hi; }
output=$(helper() { :; }; helper)
exit 0
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions();

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "helper");
    assert_eq!(
        unreached[0].reason,
        UnreachedFunctionReason::ScriptTerminates
    );
}

#[test]
fn unreached_functions_ignore_shadowed_test_argument_command_substitution_calls() {
    let source = "\
helper() { echo hi; }
if [ \"$(helper() { :; }; helper)\" = hi ]; then
  echo yes
fi
exit 0
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions();

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "helper");
    assert_eq!(
        unreached[0].reason,
        UnreachedFunctionReason::ScriptTerminates
    );
}

#[test]
fn unreached_functions_ignore_command_substitution_paths_before_later_termination() {
    let source = "\
f() { echo hi; }
output=$(echo value)
exit 0
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions();

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "f");
    assert_eq!(
        unreached[0].reason,
        UnreachedFunctionReason::ScriptTerminates
    );
}

#[test]
fn unreached_functions_ignore_pipeline_paths_before_later_termination() {
    let source = "\
f() { echo hi; }
echo value | cat
exit 0
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached = analysis.unreached_functions();

    assert_eq!(unreached.len(), 1);
    assert_eq!(unreached[0].name, "f");
    assert_eq!(
        unreached[0].reason,
        UnreachedFunctionReason::ScriptTerminates
    );
}

#[test]
fn unreached_functions_count_negated_condition_calls_before_termination() {
    let source = "\
die() { exit 1; }
check() { :; }
validate() {
  if ! check \"$value\"; then
    die
  fi
}
validate
exit 0
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached_names = analysis
        .unreached_functions()
        .iter()
        .map(|unreached| unreached.name.as_str())
        .collect::<Vec<_>>();

    assert!(!unreached_names.contains(&"check"));
}

#[test]
fn unreached_functions_count_calls_to_branch_selected_definitions() {
    let source = "\
if use_first; then
  helper() { echo first; }
else
  helper() { echo second; }
fi
run_case() {
  helper
}
run_case
exit 0
";
    let model = model(source);

    assert!(model.analysis().unreached_functions().is_empty());
}

#[test]
fn unreached_functions_count_late_bound_wrapper_calls_newer_definition() {
    let source = "\
helper() { echo old; }
run_case() {
  helper
}
helper() { echo new; }
run_case
exit 0
";
    let model = model(source);
    let analysis = model.analysis();
    let unreached_names = analysis
        .unreached_functions()
        .iter()
        .map(|unreached| unreached.name.as_str())
        .collect::<Vec<_>>();

    assert!(!unreached_names.contains(&"helper"));
}

#[test]
fn tracks_flow_context_for_conditions_and_loops() {
    let source = "\
if cmd; then
  echo ok
fi
for x in 1 2; do
  break
done
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build(&output.file, source, &indexer);

    let Command::Compound(CompoundCommand::If(if_command)) = &output.file.body[0].command else {
        panic!("expected if command");
    };
    let condition_span = match &if_command.condition[0].command {
        Command::Simple(command) => command.span,
        other => panic!("unexpected condition command: {other:?}"),
    };
    let condition_context = model.flow_context_at(&condition_span).unwrap();
    assert!(condition_context.exit_status_checked);

    let Command::Compound(CompoundCommand::For(for_command)) = &output.file.body[1].command else {
        panic!("expected for command");
    };
    let break_span = match &for_command.body[0].command {
        Command::Builtin(shuck_ast::BuiltinCommand::Break(command)) => command.span,
        other => panic!("unexpected loop body command: {other:?}"),
    };
    let break_context = model.flow_context_at(&break_span).unwrap();
    assert_eq!(break_context.loop_depth, 1);
}

#[test]
fn command_contexts_track_nested_words_and_condition_roles() {
    let source = "\
echo $(if probe; then inner; fi)
cat < <(process)
if outer; then ok; elif alt; then fallback; fi
while guard; do break; done
until stop; do done_cmd; done
while if nested_guard; then :; elif nested_alt; then :; fi; do :; done
cat <<EOF
$(heredoc_cmd)
EOF
";
    let model = model(source);

    let echo = model
        .command_context(command_id_starting_with(&model, source, "echo").unwrap())
        .unwrap();
    assert!(!echo.is_nested_word_command());
    assert_eq!(echo.nested_word_command_depth(), 0);
    assert!(echo.is_structural());
    assert_eq!(echo.condition_role(), None);

    let nested_if = model
        .command_context(command_id_starting_with(&model, source, "if probe").unwrap())
        .unwrap();
    assert!(nested_if.is_nested_word_command());
    assert_eq!(nested_if.nested_word_command_depth(), 1);
    assert_eq!(nested_if.condition_role(), None);

    let probe = model
        .command_context(command_id_starting_with(&model, source, "probe").unwrap())
        .unwrap();
    assert!(probe.is_nested_word_command());
    assert_eq!(probe.nested_word_command_depth(), 1);
    assert!(probe.is_in_if_condition());
    assert!(!probe.is_in_elif_condition());
    assert_eq!(probe.condition_role(), Some(CommandConditionRole::If));

    let process = model
        .command_context(command_id_starting_with(&model, source, "process").unwrap())
        .unwrap();
    assert!(process.is_nested_word_command());
    assert_eq!(process.nested_word_command_depth(), 1);

    let outer = model
        .command_context(command_id_starting_with(&model, source, "outer").unwrap())
        .unwrap();
    assert!(!outer.is_nested_word_command());
    assert_eq!(outer.nested_word_command_depth(), 0);
    assert!(outer.is_in_if_condition());
    assert_eq!(outer.condition_role(), Some(CommandConditionRole::If));

    let alt = model
        .command_context(command_id_starting_with(&model, source, "alt").unwrap())
        .unwrap();
    assert!(alt.is_in_if_condition());
    assert!(alt.is_in_elif_condition());
    assert_eq!(alt.condition_role(), Some(CommandConditionRole::Elif));

    let guard = model
        .command_context(command_id_starting_with(&model, source, "guard").unwrap())
        .unwrap();
    assert_eq!(guard.condition_role(), Some(CommandConditionRole::While));

    let stop = model
        .command_context(command_id_starting_with(&model, source, "stop").unwrap())
        .unwrap();
    assert_eq!(stop.condition_role(), Some(CommandConditionRole::Until));

    let nested_guard = model
        .command_context(command_id_starting_with(&model, source, "nested_guard").unwrap())
        .unwrap();
    assert_eq!(
        nested_guard.condition_role(),
        Some(CommandConditionRole::If)
    );

    let nested_alt = model
        .command_context(command_id_starting_with(&model, source, "nested_alt").unwrap())
        .unwrap();
    assert_eq!(
        nested_alt.condition_role(),
        Some(CommandConditionRole::Elif)
    );

    let heredoc = model
        .command_context(command_id_starting_with(&model, source, "heredoc_cmd").unwrap())
        .unwrap();
    assert!(heredoc.is_nested_word_command());
    assert_eq!(heredoc.nested_word_command_depth(), 1);
}

#[test]
fn condition_contexts_do_not_flow_into_function_bodies() {
    let source = "if helper() { echo body; }; then :; fi\n";
    let model = model(source);

    let helper = model
        .command_context(command_id_starting_with(&model, source, "helper").unwrap())
        .unwrap();
    assert!(helper.is_in_if_condition());
    assert_eq!(helper.condition_role(), Some(CommandConditionRole::If));

    let body = model
        .command_context(command_id_starting_with(&model, source, "echo body").unwrap())
        .unwrap();
    assert!(!body.is_in_if_condition());
    assert!(!body.is_in_elif_condition());
    assert_eq!(body.condition_role(), None);
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
fn previous_visible_binding_can_ignore_the_current_assignment_span() {
    let model = model(
        "\
#!/bin/bash
value=outer
f() {
  local value=inner
  value=next
}
",
    );
    let binding_ids = model.bindings_for(&Name::from("value"));
    let local = model.binding(binding_ids[1]);
    let current = model.binding(binding_ids[2]);

    assert_eq!(
        model
            .visible_binding(&Name::from("value"), current.span)
            .map(|binding| binding.id),
        Some(current.id)
    );
    assert_eq!(
        model
            .previous_visible_binding(&Name::from("value"), current.span, Some(current.span))
            .map(|binding| binding.id),
        Some(local.id)
    );
}

#[test]
fn visible_candidate_bindings_for_reference_returns_visible_scope_chain() {
    let source = "\
#!/bin/bash
value=outer
f() {
  local value=inner
  printf '%s\\n' \"$value\"
}
";
    let model = model(source);
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "value")
        .unwrap();
    let candidates = model.visible_candidate_bindings_for_reference(reference);
    let value_bindings = model.bindings_for(&Name::from("value"));

    assert_eq!(candidates, vec![value_bindings[1], value_bindings[0]]);
}

#[test]
fn visible_candidate_bindings_for_reference_falls_back_to_prior_outer_scope_bindings() {
    let source = "\
#!/bin/bash
first() {
  target=(one two)
}
second() {
  printf '%s\\n' \"$target\"
}
";
    let model = model(source);
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "target")
        .unwrap();
    let candidates = model.visible_candidate_bindings_for_reference(reference);

    assert_eq!(candidates, model.bindings_for(&Name::from("target")));
}

#[test]
fn assoc_lookup_binding_prefers_visible_assoc_declarations_and_respects_local_shadowing() {
    let model = model(
        "\
#!/bin/bash
declare -A map
helper() {
  map[$key]=1
}
shadow() {
  local map
  map[$key]=2
}
",
    );
    let helper_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "helper") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .expect("expected helper scope");
    let shadow_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "shadow") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .expect("expected shadow scope");
    let helper_assignment = model
        .bindings_for(&Name::from("map"))
        .iter()
        .copied()
        .find(|binding_id| {
            let binding = model.binding(*binding_id);
            binding.scope == helper_scope && binding.kind == BindingKind::ArrayAssignment
        })
        .expect("expected helper assignment binding");
    let shadow_local = model
        .bindings_for(&Name::from("map"))
        .iter()
        .copied()
        .find(|binding_id| {
            let binding = model.binding(*binding_id);
            binding.scope == shadow_scope
                && matches!(binding.kind, BindingKind::Declaration(_))
                && binding.attributes.contains(BindingAttributes::LOCAL)
        })
        .expect("expected shadowing local binding");
    let shadow_assignment = model
        .bindings_for(&Name::from("map"))
        .iter()
        .copied()
        .find(|binding_id| {
            let binding = model.binding(*binding_id);
            binding.scope == shadow_scope && binding.kind == BindingKind::ArrayAssignment
        })
        .expect("expected shadow assignment binding");

    assert!(
        model
            .visible_binding_for_assoc_lookup(
                &Name::from("map"),
                helper_scope,
                model.binding(helper_assignment).span,
            )
            .is_some_and(|binding| binding.attributes.contains(BindingAttributes::ASSOC))
    );
    assert_eq!(
        model
            .visible_binding_for_assoc_lookup(
                &Name::from("map"),
                shadow_scope,
                model.binding(shadow_assignment).span,
            )
            .map(|binding| binding.id),
        Some(shadow_local)
    );
}

#[test]
fn imported_entry_bindings_insert_in_visibility_order() {
    let source = "\
#!/bin/bash
value=local
echo $value
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build_with_options(
        &output.file,
        source,
        &indexer,
        SemanticBuildOptions {
            file_entry_contract: Some(FileContract {
                required_reads: Vec::new(),
                provided_bindings: vec![ProvidedBinding::new(
                    Name::from("value"),
                    ProvidedBindingKind::Variable,
                    ContractCertainty::Definite,
                )],
                provided_functions: Vec::new(),
                externally_consumed_bindings: false,
            }),
            ..SemanticBuildOptions::default()
        },
    );
    let value_bindings = model.bindings_for(&Name::from("value"));

    assert_eq!(value_bindings.len(), 2);
    assert!(matches!(
        model.binding(value_bindings[0]).kind,
        BindingKind::Imported
    ));
    assert!(matches!(
        model.binding(value_bindings[1]).kind,
        BindingKind::Assignment
    ));

    let value_reference = model
        .references()
        .iter()
        .find(|reference| {
            reference.name.as_str() == "value" && reference.span.slice(source) == "$value"
        })
        .expect("expected value reference");

    assert!(matches!(
        model.visible_binding(&Name::from("value"), value_reference.span),
        Some(binding) if binding.kind == BindingKind::Assignment
    ));
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

#[test]
fn name_only_export_consumes_existing_binding() {
    let source = "foo=1\nexport foo\n";
    let model = model(source);

    let foo_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo")
        .collect::<Vec<_>>();
    assert_eq!(foo_bindings.len(), 1);
    assert!(
        foo_bindings[0]
            .attributes
            .contains(BindingAttributes::EXPORTED)
    );

    let declaration_reference = model
        .references()
        .iter()
        .find(|reference| {
            reference.kind == ReferenceKind::DeclarationName && reference.name == "foo"
        })
        .unwrap();
    let resolved = model.resolved_binding(declaration_reference.id).unwrap();
    assert_eq!(resolved.id, foo_bindings[0].id);
}

#[test]
fn name_only_local_creates_a_binding_for_later_reads() {
    let source = "f() { local VAR; echo \"$VAR\"; }\n";
    let model = model(source);

    let local_binding = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "VAR"
                && matches!(
                    binding.kind,
                    BindingKind::Declaration(DeclarationBuiltin::Local)
                )
        })
        .unwrap();
    assert!(
        !local_binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
    );

    let reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "VAR")
        .unwrap();
    let resolved = model.resolved_binding(reference.id).unwrap();
    assert_eq!(resolved.id, local_binding.id);
    let reference_id = reference.id;
    let analysis = model.analysis();
    let uninitialized = analysis.uninitialized_references();
    assert_eq!(uninitialized.len(), 1);
    assert_eq!(uninitialized[0].reference, reference_id);
    assert_eq!(uninitialized[0].certainty, UninitializedCertainty::Definite);
}

#[test]
fn declaration_function_name_operands_do_not_create_variable_bindings() {
    let source = "\
declare -f -F config_unset >/dev/null
export -f helper
readonly -f locked
typeset -f typed
";
    let model = model(source);
    let function_operand_names = ["config_unset", "helper", "locked", "typed"];

    for name in function_operand_names {
        assert!(
            model.bindings().iter().all(|binding| binding.name != name),
            "{name} should be treated as a function name, not a variable binding"
        );
        assert!(
            model.references().iter().all(|reference| {
                reference.name != name || reference.kind != ReferenceKind::DeclarationName
            }),
            "{name} should not create a declaration-name variable reference"
        );
    }
}

#[test]
fn declaration_plus_function_flags_keep_name_operands_as_variables() {
    let source = "\
declare +f config_unset
declare +F config_maybe
declare -f +f config_after_toggle
declare +f config_before -f helper_function
declare -f hidden_function +f config_after
typeset +f typed
";
    let model = model(source);
    let variable_operand_names = [
        "config_unset",
        "config_maybe",
        "config_after_toggle",
        "config_before",
        "config_after",
        "typed",
    ];

    for name in variable_operand_names {
        assert!(
            model.bindings().iter().any(|binding| binding.name == name
                && matches!(binding.kind, BindingKind::Declaration(_))),
            "{name} should be treated as a variable declaration"
        );
    }

    let function_operand_names = ["helper_function", "hidden_function"];
    for name in function_operand_names {
        assert!(
            model.bindings().iter().all(|binding| binding.name != name),
            "{name} should be treated as a function name, not a variable binding"
        );
        assert!(
            model.references().iter().all(|reference| {
                reference.name != name || reference.kind != ReferenceKind::DeclarationName
            }),
            "{name} should not create a declaration-name variable reference"
        );
    }
}

#[test]
fn repeated_name_only_local_reuses_existing_same_scope_binding() {
    let source = "f() { local foo=1; local foo; echo \"$foo\"; }\n";
    let model = model(source);

    let foo_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo")
        .collect::<Vec<_>>();
    assert_eq!(foo_bindings.len(), 1);
    assert!(
        foo_bindings[0]
            .attributes
            .contains(BindingAttributes::LOCAL)
    );
    assert!(
        foo_bindings[0]
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
    );

    let declaration_reference = model
        .references()
        .iter()
        .find(|reference| {
            reference.kind == ReferenceKind::DeclarationName && reference.name == "foo"
        })
        .unwrap();
    let resolved_declaration = model.resolved_binding(declaration_reference.id).unwrap();
    assert_eq!(resolved_declaration.id, foo_bindings[0].id);

    let expansion_reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "foo")
        .unwrap();
    let resolved_expansion = model.resolved_binding(expansion_reference.id).unwrap();
    assert_eq!(resolved_expansion.id, foo_bindings[0].id);

    let analysis = model.analysis();
    assert!(analysis.uninitialized_references().is_empty());
    assert!(analysis.unused_assignments().is_empty());
}

#[test]
fn name_only_local_after_local_write_reuses_existing_local_state() {
    let source = "f() { local foo=1; foo=2; local foo; echo \"$foo\"; }\n";
    let model = model(source);

    let foo_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo")
        .collect::<Vec<_>>();
    assert_eq!(foo_bindings.len(), 2);
    assert!(
        foo_bindings[0]
            .attributes
            .contains(BindingAttributes::LOCAL)
    );
    assert!(
        foo_bindings[1]
            .attributes
            .contains(BindingAttributes::LOCAL)
    );

    let declaration_reference = model
        .references()
        .iter()
        .find(|reference| {
            reference.kind == ReferenceKind::DeclarationName && reference.name == "foo"
        })
        .unwrap();
    let resolved_declaration = model.resolved_binding(declaration_reference.id).unwrap();
    assert_eq!(resolved_declaration.id, foo_bindings[1].id);

    let expansion_reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "foo")
        .unwrap();
    let resolved_expansion = model.resolved_binding(expansion_reference.id).unwrap();
    assert_eq!(resolved_expansion.id, foo_bindings[1].id);

    let unused_binding_ids = model.analysis().dataflow().unused_assignment_ids().to_vec();
    assert_eq!(unused_binding_ids, vec![foo_bindings[0].id]);
    assert!(model.analysis().uninitialized_references().is_empty());
}

#[test]
fn name_only_local_after_conditional_unset_reuses_existing_local_state() {
    let source = "\
f() {
  local foo=1
  if cond; then
    unset foo
  fi
  local foo
  echo \"$foo\"
}
";
    let model = model(source);
    let function_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "f") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .expect("expected function scope");

    let foo_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && binding.scope == function_scope)
        .collect::<Vec<_>>();
    assert_eq!(foo_bindings.len(), 1);
    assert!(
        foo_bindings[0]
            .attributes
            .contains(BindingAttributes::LOCAL)
    );

    let expansion_reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "foo")
        .expect("expected foo expansion");
    let resolved_expansion = model.resolved_binding(expansion_reference.id).unwrap();
    assert_eq!(resolved_expansion.id, foo_bindings[0].id);

    let analysis = model.analysis();
    assert!(analysis.uninitialized_references().is_empty());
    assert!(analysis.unused_assignments().is_empty());
}

#[test]
fn name_only_local_after_invalid_unset_flag_reuses_existing_local_state() {
    let source = "\
f() {
  local foo=1
  unset -z foo
  local foo
  echo \"$foo\"
}
";
    let model = model(source);
    let function_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "f") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .expect("expected function scope");

    let foo_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && binding.scope == function_scope)
        .collect::<Vec<_>>();
    assert_eq!(foo_bindings.len(), 1);
    assert!(
        foo_bindings[0]
            .attributes
            .contains(BindingAttributes::LOCAL)
    );

    let expansion_reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "foo")
        .expect("expected foo expansion");
    let resolved_expansion = model.resolved_binding(expansion_reference.id).unwrap();
    assert_eq!(resolved_expansion.id, foo_bindings[0].id);

    let analysis = model.analysis();
    assert!(analysis.uninitialized_references().is_empty());
    assert!(analysis.unused_assignments().is_empty());
}

#[test]
fn name_only_local_after_conflicting_unset_flags_reuses_existing_local_state() {
    let source = "\
f() {
  local foo=1
  unset -vf foo
  local foo
  echo \"$foo\"
}
";
    let model = model(source);
    let function_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "f") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .expect("expected function scope");

    let foo_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && binding.scope == function_scope)
        .collect::<Vec<_>>();
    assert_eq!(foo_bindings.len(), 1);
    assert!(
        foo_bindings[0]
            .attributes
            .contains(BindingAttributes::LOCAL)
    );

    let expansion_reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "foo")
        .expect("expected foo expansion");
    let resolved_expansion = model.resolved_binding(expansion_reference.id).unwrap();
    assert_eq!(resolved_expansion.id, foo_bindings[0].id);

    let analysis = model.analysis();
    assert!(analysis.uninitialized_references().is_empty());
    assert!(analysis.unused_assignments().is_empty());
}

#[test]
fn name_only_local_after_split_conflicting_unset_flags_reuses_existing_local_state() {
    let source = "\
f() {
  local foo=1
  unset -f -v foo
  local foo
  echo \"$foo\"
}
";
    let model = model(source);
    let function_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "f") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .expect("expected function scope");

    let foo_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && binding.scope == function_scope)
        .collect::<Vec<_>>();
    assert_eq!(foo_bindings.len(), 1);
    assert!(
        foo_bindings[0]
            .attributes
            .contains(BindingAttributes::LOCAL)
    );

    let expansion_reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "foo")
        .expect("expected foo expansion");
    let resolved_expansion = model.resolved_binding(expansion_reference.id).unwrap();
    assert_eq!(resolved_expansion.id, foo_bindings[0].id);

    let analysis = model.analysis();
    assert!(analysis.uninitialized_references().is_empty());
    assert!(analysis.unused_assignments().is_empty());
}

#[test]
fn unset_n_clears_nameref_binding_state_before_name_only_local() {
    let source = "\
f() {
  local foo=1
  local -n ref=foo
  unset -n ref
  local ref
}
";
    let model = model(source);
    let function_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "f") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .expect("expected function scope");

    let ref_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "ref" && binding.scope == function_scope)
        .collect::<Vec<_>>();

    assert!(
        ref_bindings
            .iter()
            .any(|binding| matches!(binding.kind, BindingKind::Nameref))
    );
    let redeclared_local = ref_bindings
        .iter()
        .find(|binding| {
            matches!(
                binding.kind,
                BindingKind::Declaration(DeclarationBuiltin::Local)
            )
        })
        .expect("expected fresh local binding after unset -n");
    assert!(
        !redeclared_local
            .attributes
            .contains(BindingAttributes::NAMEREF)
    );
}

#[test]
fn name_only_local_after_unset_n_plain_variable_reuses_existing_local_state() {
    let source = "\
f() {
  local foo=1
  unset -n foo
  local foo
  echo \"$foo\"
}
";
    let model = model(source);
    let function_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "f") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .expect("expected function scope");

    let foo_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && binding.scope == function_scope)
        .collect::<Vec<_>>();
    assert_eq!(foo_bindings.len(), 1);
    assert!(
        foo_bindings[0]
            .attributes
            .contains(BindingAttributes::LOCAL)
    );

    let expansion_reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "foo")
        .expect("expected foo expansion");
    let resolved_expansion = model.resolved_binding(expansion_reference.id).unwrap();
    assert_eq!(resolved_expansion.id, foo_bindings[0].id);

    let analysis = model.analysis();
    assert!(analysis.uninitialized_references().is_empty());
    assert!(analysis.unused_assignments().is_empty());
}

#[test]
fn name_only_local_after_dynamic_unset_option_word_reuses_existing_local_state() {
    let source = "\
f() {
  local foo=1
  local mode=-f
  unset \"$mode\" foo
  local foo
  echo \"$foo\"
}
";
    let model = model(source);
    let function_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "f") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .expect("expected function scope");

    let foo_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && binding.scope == function_scope)
        .collect::<Vec<_>>();
    assert_eq!(foo_bindings.len(), 1);
    assert!(
        foo_bindings[0]
            .attributes
            .contains(BindingAttributes::LOCAL)
    );

    let expansion_reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "foo")
        .expect("expected foo expansion");
    let resolved_expansion = model.resolved_binding(expansion_reference.id).unwrap();
    assert_eq!(resolved_expansion.id, foo_bindings[0].id);

    let analysis = model.analysis();
    assert!(analysis.uninitialized_references().is_empty());
    assert!(analysis.unused_assignments().is_empty());
}

#[test]
fn name_only_local_after_unset_creates_fresh_non_assoc_binding() {
    let source = "\
main() {
  local key=name
  declare -A map
  unset map
  local map
  map[$key]=1
}
";
    let model = model(source);
    let main_scope = model
        .scopes()
        .iter()
        .find_map(|scope| match &scope.kind {
            ScopeKind::Function(FunctionScopeKind::Named(names))
                if names.iter().any(|name| name == "main") =>
            {
                Some(scope.id)
            }
            _ => None,
        })
        .expect("expected main scope");
    let redeclared_local = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "map"
                && binding.scope == main_scope
                && matches!(
                    binding.kind,
                    BindingKind::Declaration(DeclarationBuiltin::Local)
                )
        })
        .expect("expected redeclared local binding");
    let array_assignment = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "map"
                && binding.scope == main_scope
                && binding.kind == BindingKind::ArrayAssignment
        })
        .expect("expected array assignment binding");

    assert!(
        !redeclared_local
            .attributes
            .contains(BindingAttributes::ASSOC)
    );
    assert_eq!(
            model
                .visible_binding_for_assoc_lookup(
                    &Name::from("map"),
                    main_scope,
                    array_assignment.span,
                )
                .map(|binding| binding.id),
            Some(redeclared_local.id)
        );
}

#[test]
fn special_command_targets_store_name_only_spans() {
    let source = "\
read -r read_target
read -ra read_array_target read_array_remainder
read -aattached_target
read -ar
mapfile mapfile_target
readarray readarray_target
printf -v printf_target '%s' value
getopts 'ab' getopts_target
";
    let model = model(source);

    let read_target = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "read_target" && matches!(binding.kind, BindingKind::ReadTarget)
        })
        .unwrap();
    assert_eq!(read_target.span.slice(source), "read_target");
    assert!(!read_target.attributes.contains(BindingAttributes::ARRAY));

    let read_array_target = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "read_array_target" && matches!(binding.kind, BindingKind::ReadTarget)
        })
        .unwrap();
    assert_eq!(read_array_target.span.slice(source), "read_array_target");
    assert!(
        read_array_target
            .attributes
            .contains(BindingAttributes::ARRAY)
    );
    assert!(!model.bindings().iter().any(|binding| {
        binding.name == "read_array_remainder" && matches!(binding.kind, BindingKind::ReadTarget)
    }));

    let attached_read_target = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "attached_target" && matches!(binding.kind, BindingKind::ReadTarget)
        })
        .unwrap();
    assert_eq!(attached_read_target.span.slice(source), "attached_target");

    let short_attached_read_target = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "r" && matches!(binding.kind, BindingKind::ReadTarget))
        .unwrap();
    assert_eq!(short_attached_read_target.span.slice(source), "r");

    let mapfile_target = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "mapfile_target" && matches!(binding.kind, BindingKind::MapfileTarget)
        })
        .unwrap();
    assert_eq!(mapfile_target.span.slice(source), "mapfile_target");
    assert!(mapfile_target.attributes.contains(BindingAttributes::ARRAY));

    let readarray_target = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "readarray_target" && matches!(binding.kind, BindingKind::MapfileTarget)
        })
        .unwrap();
    assert_eq!(readarray_target.span.slice(source), "readarray_target");
    assert!(
        readarray_target
            .attributes
            .contains(BindingAttributes::ARRAY)
    );

    let printf_target = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "printf_target" && matches!(binding.kind, BindingKind::PrintfTarget)
        })
        .unwrap();
    assert_eq!(printf_target.span.slice(source), "printf_target");

    let getopts_target = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "getopts_target" && matches!(binding.kind, BindingKind::GetoptsTarget)
        })
        .unwrap();
    assert_eq!(getopts_target.span.slice(source), "getopts_target");
}

#[test]
fn special_command_target_parsing_skips_option_operands_and_tracks_implicit_mapfile() {
    let source = "\
delimiter=:
callback=cb
read -d delimiter -a read_array_target read_array_remainder <<<\":\"
mapfile -C callback -c 1 mapfile_target < <(printf '%s\\n' value)
mapfile
";
    let model = model(source);

    let read_targets = model
        .bindings()
        .iter()
        .filter(|binding| matches!(binding.kind, BindingKind::ReadTarget))
        .collect::<Vec<_>>();
    assert!(
        read_targets
            .iter()
            .any(|binding| binding.name == "read_array_target")
    );
    assert!(
        !read_targets
            .iter()
            .any(|binding| binding.name == "read_array_remainder")
    );
    assert!(
        !read_targets
            .iter()
            .any(|binding| binding.name == "delimiter")
    );

    let mapfile_targets = model
        .bindings()
        .iter()
        .filter(|binding| matches!(binding.kind, BindingKind::MapfileTarget))
        .collect::<Vec<_>>();
    assert!(
        mapfile_targets
            .iter()
            .any(|binding| binding.name == "mapfile_target")
    );
    assert!(mapfile_targets.iter().any(|binding| {
        binding.name == "MAPFILE"
            && binding.attributes.contains(BindingAttributes::ARRAY)
            && matches!(
                binding.origin,
                BindingOrigin::BuiltinTarget {
                    definition_span,
                    ..
                } if definition_span == binding.span
            )
    }));
    assert!(
        !mapfile_targets
            .iter()
            .any(|binding| binding.name == "callback")
    );
}

#[test]
fn mapfile_missing_option_operand_does_not_panic() {
    let source = "mapfile -u\n";
    let model = model(source);

    assert!(
        !model
            .bindings()
            .iter()
            .any(|binding| matches!(binding.kind, BindingKind::MapfileTarget))
    );
}

#[test]
fn read_header_bindings_consumed_in_loop_body_are_live() {
    let source = "\
printf '%s\n' 'service safe ok yes' | while read UNIT EXPOSURE PREDICATE HAPPY; do
  printf '%s %s %s %s\n' \"$UNIT\" \"$EXPOSURE\" \"$PREDICATE\" \"$HAPPY\"
done
";
    let model = model(source);
    let unused = reportable_unused_names(&model);

    for name in ["UNIT", "EXPOSURE", "PREDICATE", "HAPPY"] {
        assert!(
            !unused.contains(&Name::from(name)),
            "unused bindings: {:?}",
            unused
        );
    }
}

#[test]
fn command_prefix_assignments_do_not_create_shell_bindings() {
    let source = "\
base_flags=1
CFLAGS=\"$base_flags\" make
echo \"$CFLAGS\"
";
    let model = model(source);

    assert!(
        model
            .bindings()
            .iter()
            .all(|binding| binding.name != "CFLAGS")
    );

    let cflags_reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "CFLAGS")
        .unwrap();
    assert!(model.resolved_binding(cflags_reference.id).is_none());
    assert!(model.unresolved_references().contains(&cflags_reference.id));
}

#[test]
fn indirect_expansion_keeps_dynamic_target_arrays_live() {
    let source = "\
#!/bin/bash
apache_args=(--apache)
nginx_args=(--nginx)
apache_args+=(--common)
nginx_args+=(--common)
web_server=apache
args_var=\"${web_server}_args[@]\"
printf '%s\\n' \"${!args_var}\"
";
    let model = model(source);
    model.analysis().dataflow();

    let unused = model
        .analysis()
        .unused_assignments()
        .iter()
        .map(|binding| model.binding(*binding).name.as_str())
        .collect::<Vec<_>>();
    assert!(!unused.contains(&"apache_args"));
    assert!(!unused.contains(&"nginx_args"));

    let carrier = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "args_var")
        .unwrap();
    let reference = model
        .references()
        .iter()
        .find(|reference| {
            reference.kind == ReferenceKind::IndirectExpansion && reference.name == "args_var"
        })
        .unwrap();

    let mut carrier_targets = binding_names(&model, model.indirect_targets_for_binding(carrier.id));
    carrier_targets.sort();
    carrier_targets.dedup();
    assert_eq!(carrier_targets, vec!["apache_args", "nginx_args"]);

    let mut reference_targets =
        binding_names(&model, model.indirect_targets_for_reference(reference.id));
    reference_targets.sort();
    reference_targets.dedup();
    assert_eq!(reference_targets, vec!["apache_args", "nginx_args"]);
}

#[test]
fn append_assignments_contribute_to_later_array_expansion() {
    let source = "\
#!/bin/bash
arr=(--first)
arr+=(--second)
printf '%s\\n' \"${arr[@]}\"
";
    let model = model(source);
    model.analysis().dataflow();

    let unused = model
        .analysis()
        .unused_assignments()
        .iter()
        .map(|binding| model.binding(*binding).name.as_str())
        .collect::<Vec<_>>();
    assert!(!unused.contains(&"arr"));
}

#[test]
fn process_substitution_reads_keep_outer_assignments_live() {
    let source = "\
#!/bin/bash
f() {
  local opts
  case \"$1\" in
    a) opts=alpha ;;
    *) opts=beta ;;
  esac
  while IFS= read -r line; do :; done < <(printf '%s\\n' \"$opts\")
}
f a
";
    let model = model(source);
    let unused = reportable_unused_names(&model);

    assert!(
        !unused.contains(&Name::from("opts")),
        "unused: {:?}",
        unused
    );
    assert!(unused.contains(&Name::from("line")), "unused: {:?}", unused);
}

#[test]
fn overwritten_empty_initializers_do_not_report_the_placeholder_assignment() {
    let plain = model(
        "\
#!/bin/bash
foo=
foo=bar
",
    );
    let plain_unused = plain.analysis().unused_assignments().to_vec();
    assert_eq!(plain_unused.len(), 1);
    assert_eq!(plain.binding(plain_unused[0]).span.start.line, 3);

    let local = model(
        "\
#!/bin/bash
f() {
  local foo=
  foo=bar
}
f
",
    );
    let local_unused = local.analysis().unused_assignments().to_vec();
    assert_eq!(local_unused.len(), 1);
    assert_eq!(local.binding(local_unused[0]).span.start.line, 4);
}

#[test]
fn associative_compound_declaration_marks_binding_assoc_and_array() {
    let model = model("#!/bin/bash\ndeclare -A assoc=(one [foo]=bar [bar]+=baz)\n");

    let assoc = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "assoc")
        .expect("expected assoc binding");
    assert!(assoc.attributes.contains(BindingAttributes::ARRAY));
    assert!(assoc.attributes.contains(BindingAttributes::ASSOC));
}

#[test]
fn read_implicitly_consumes_visible_ifs_binding() {
    let source = "\
#!/bin/bash
f() {
  local IFS=$'\\n'
  local unused=1
  read -d '' -ra reply < <(printf 'alpha\\nbeta\\0')
  printf '%s\\n' \"${reply[@]}\"
}
f
";
    let model = model(source);
    model.analysis().dataflow();

    assert!(model.references().iter().any(|reference| {
        reference.name == "IFS" && matches!(reference.kind, ReferenceKind::ImplicitRead)
    }));

    let unused = model
        .analysis()
        .unused_assignments()
        .iter()
        .map(|binding| model.binding(*binding).name.as_str())
        .collect::<Vec<_>>();
    assert!(!unused.contains(&"IFS"));
    assert!(unused.contains(&"unused"));
}

#[test]
fn reaching_bindings_lookup_picks_later_same_name_reference() {
    let source = "\
#!/bin/bash
foo=1
printf '%s\\n' \"$foo\"
foo=2
printf '%s\\n' \"$foo\"
";
    let model = model(source);
    let foo_bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .map(|binding| binding.id)
        .collect::<Vec<_>>();
    assert_eq!(foo_bindings.len(), 2);

    let target_reference = model
        .references()
        .iter()
        .rev()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let reaching = model
        .analysis()
        .reaching_bindings_for_name(&target_reference.name, target_reference.span);

    assert_eq!(reaching, vec![foo_bindings[1]]);
}

#[test]
fn semantic_analysis_exposes_binding_and_reference_flow_blocks() {
    let source = "\
#!/bin/bash
foo=1
printf '%s\\n' \"$foo\"
";
    let model = model(source);
    let binding = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .expect("expected foo binding");
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();

    assert_eq!(
        analysis.reference_id_for_name_at(&reference.name, reference.span),
        Some(reference.id)
    );
    assert!(analysis.block_for_binding(binding.id).is_some());
    assert!(analysis.block_for_reference_id(reference.id).is_some());
    assert!(analysis.binding_dominates_reference_from_flow_entry(
        binding.id,
        &reference.name,
        reference.span,
        true
    ));
}

#[test]
fn semantic_block_coverage_detects_uncovered_branch_paths() {
    let source = "\
#!/bin/bash
if cond; then
  foo=1
fi
printf '%s\\n' \"$foo\"
";
    let model = model(source);
    let binding = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .expect("expected foo binding");
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();
    let binding_block = analysis
        .block_for_binding(binding.id)
        .expect("expected binding block");
    let reference_block = analysis
        .block_for_reference_id(reference.id)
        .expect("expected reference block");
    let entry =
        analysis.flow_entry_block_for_binding_scopes(&[binding.scope], reference.span.start.offset);
    let cover_blocks = FxHashSet::from_iter([binding_block]);

    assert!(!analysis.blocks_cover_all_paths_to_block(entry, reference_block, &cover_blocks));
    assert!(!analysis.binding_dominates_reference_from_flow_entry(
        binding.id,
        &reference.name,
        reference.span,
        true
    ));
}

#[test]
fn value_flow_returns_real_reference_reaching_value_bindings() {
    let source = "\
#!/bin/bash
foo=1
foo=2
printf '%s\\n' \"$foo\"
";
    let model = model(source);
    let bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .map(|binding| binding.id)
        .collect::<Vec<_>>();
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();
    let value_flow = analysis.value_flow();

    assert_eq!(
        value_flow.reaching_value_bindings_for_name(&reference.name, reference.span),
        vec![bindings[1]]
    );
}

#[test]
fn value_flow_returns_synthetic_use_site_value_bindings() {
    let source = "\
#!/bin/bash
foo=1
if cond; then
  foo=2
fi
printf done
";
    let model = model(source);
    let printf = command_id_starting_with(&model, source, "printf").expect("expected printf");
    let bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .map(|binding| binding.id)
        .collect::<Vec<_>>();
    let analysis = model.analysis();
    let value_flow = analysis.value_flow();

    assert_eq!(
        value_flow.reaching_value_bindings_for_name(&Name::from("foo"), model.command_span(printf)),
        bindings
    );
}

#[test]
fn value_flow_can_bypass_one_reaching_binding() {
    let source = "\
#!/bin/bash
foo=1
if cond; then
  foo=2
fi
printf '%s\\n' \"$foo\"
";
    let model = model(source);
    let bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .map(|binding| binding.id)
        .collect::<Vec<_>>();
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();
    let value_flow = analysis.value_flow();

    assert_eq!(
        value_flow.reaching_value_bindings_bypassing(&reference.name, bindings[1], reference.span),
        vec![bindings[0]]
    );
}

#[test]
fn value_flow_returns_ancestor_bindings_before_scope_site() {
    let source = "\
#!/bin/bash
foo=1
use() {
  printf '%s\\n' \"$foo\"
}
";
    let model = model(source);
    let binding = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .expect("expected foo binding");
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();
    let value_flow = analysis.value_flow();

    assert_eq!(
        value_flow.ancestor_value_bindings_before(&reference.name, reference.scope, reference.span),
        vec![binding.id]
    );
}

#[test]
fn value_flow_returns_nonlocal_bindings_from_called_functions() {
    let source = "\
#!/bin/bash
setfoo() {
  foo=1
}
use() {
  setfoo
  printf '%s\\n' \"$foo\"
}
";
    let model = model(source);
    let binding = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .expect("expected foo binding");
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();
    let mut value_flow = analysis.value_flow();

    assert_eq!(
        value_flow.nonlocal_value_bindings_from_called_functions_before(
            &reference.name,
            reference.scope,
            reference.span,
        ),
        vec![binding.id]
    );
}

#[test]
fn value_flow_ignores_conditionally_installed_called_functions() {
    let source = "\
#!/bin/bash
if cond; then
  setfoo() {
    foo=1
  }
fi
use() {
  setfoo
  printf '%s\\n' \"$foo\"
}
";
    let model = model(source);
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();
    let mut value_flow = analysis.value_flow();

    assert!(
        value_flow
            .nonlocal_value_bindings_from_called_functions_before(
                &reference.name,
                reference.scope,
                reference.span,
            )
            .is_empty()
    );
}

#[test]
fn value_flow_uses_visible_top_level_functions_from_function_bodies() {
    let source = "\
#!/bin/bash
use() {
  setfoo
  printf '%s\\n' \"$foo\"
}
setfoo() {
  foo=1
}
use
";
    let model = model(source);
    let binding = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .expect("expected foo binding");
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();
    let mut value_flow = analysis.value_flow();

    assert_eq!(
        value_flow.nonlocal_value_bindings_from_called_functions_before(
            &reference.name,
            reference.scope,
            reference.span,
        ),
        vec![binding.id]
    );
}

#[test]
fn value_flow_resolves_multi_name_function_alias_fallbacks() {
    let source = "\
#!/bin/zsh
use() {
  itunes
  print $foo
}
function music itunes() {
  foo=1
}
use
";
    let model = model_with_dialect(source, ShellDialect::Zsh);
    let binding = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .expect("expected foo binding");
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();
    let mut value_flow = analysis.value_flow();

    assert_eq!(
        value_flow.nonlocal_value_bindings_from_called_functions_before(
            &reference.name,
            reference.scope,
            reference.span,
        ),
        vec![binding.id]
    );
}

#[test]
fn value_flow_returns_helper_bindings_visible_at_reference() {
    let source = "\
#!/bin/bash
setfoo() {
  foo=1
}
use() {
  setfoo
  printf '%s\\n' \"$foo\"
}
";
    let model = model(source);
    let binding = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .expect("expected foo binding");
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();
    let mut value_flow = analysis.value_flow();

    assert_eq!(
        value_flow.helper_value_bindings_before(&reference.name, reference.span),
        vec![binding.id]
    );
}

#[test]
fn value_flow_named_function_call_sites_resolve_alias_fallbacks() {
    let source = "\
#!/bin/zsh
use() {
  itunes
  print $foo
}
function music itunes() {
  foo=1
}
use
";
    let model = model_with_dialect(source, ShellDialect::Zsh);
    let helper_scope = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .map(|binding| binding.scope)
        .expect("expected helper scope");
    let analysis = model.analysis();
    let value_flow = analysis.value_flow();
    let call_sites = value_flow.named_function_call_sites(helper_scope);

    assert_eq!(call_sites.len(), 1);
    assert_eq!(call_sites[0].callee.as_str(), "itunes");
}

#[test]
fn value_flow_tracks_transitive_called_function_scopes_before() {
    let source = "\
#!/bin/bash
first() {
  second
}
second() {
  foo=1
}
first
printf '%s\\n' \"$foo\"
";
    let model = model(source);
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let helper_scope = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .map(|binding| binding.scope)
        .expect("expected helper scope");
    let analysis = model.analysis();
    let value_flow = analysis.value_flow();
    let caller_scope = model.scope_at(reference.span.start.offset);
    let callee_scopes = value_flow
        .transitively_called_function_scopes_before(caller_scope, reference.span.start.offset);

    assert!(callee_scopes.contains(&helper_scope));
}

#[test]
fn value_flow_uses_path_covering_alternate_function_definitions() {
    let source = "\
#!/bin/bash
if cond; then
  setfoo() {
    foo=1
  }
else
  setfoo() {
    foo=2
  }
fi
use() {
  setfoo
  printf '%s\\n' \"$foo\"
}
";
    let model = model(source);
    let bindings = model
        .bindings()
        .iter()
        .filter(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .map(|binding| binding.id)
        .collect::<Vec<_>>();
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "foo")
        .expect("expected foo reference");
    let analysis = model.analysis();
    let mut value_flow = analysis.value_flow();

    assert_eq!(
        value_flow.nonlocal_value_bindings_from_called_functions_before(
            &reference.name,
            reference.scope,
            reference.span,
        ),
        bindings
    );
}

#[test]
fn value_flow_detects_binding_paths_do_not_cover_span() {
    let source = "\
#!/bin/bash
if cond; then
  foo=1
fi
printf '%s\\n' \"$foo\"
";
    let model = model(source);
    let binding = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "foo" && matches!(binding.kind, BindingKind::Assignment))
        .expect("expected foo binding");
    let printf = command_id_starting_with(&model, source, "printf").expect("expected printf");
    let analysis = model.analysis();
    let value_flow = analysis.value_flow();

    assert!(
        !value_flow
            .value_bindings_cover_all_paths_to_span(&[binding.id], model.command_span(printf))
    );
}

#[test]
fn ifs_assignments_are_treated_as_implicitly_used() {
    let source = "\
#!/bin/bash
IFS=$'\\n\\t'
unused=1
echo ok
";
    let model = model(source);
    model.analysis().dataflow();

    let unused = model
        .analysis()
        .unused_assignments()
        .iter()
        .map(|binding| model.binding(*binding).name.as_str())
        .collect::<Vec<_>>();
    assert!(!unused.contains(&"IFS"));
    assert!(unused.contains(&"unused"));
}

#[test]
fn shell_runtime_assignments_are_treated_as_implicitly_used() {
    let source = "\
#!/bin/sh
PATH=$PATH:/opt/custom
CDPATH=/tmp
LANG=C
LC_ALL=C
LC_TIME=C
unused=1
echo ok
";
    let model = model(source);
    model.analysis().dataflow();

    let unused = model
        .analysis()
        .unused_assignments()
        .iter()
        .map(|binding| model.binding(*binding).name.as_str())
        .collect::<Vec<_>>();
    for name in ["PATH", "CDPATH", "LANG", "LC_ALL", "LC_TIME"] {
        assert!(!unused.contains(&name), "unused bindings: {:?}", unused);
    }
    assert!(unused.contains(&"unused"));
}

#[test]
fn special_runtime_assignments_are_treated_as_implicitly_used() {
    let source = "\
#!/bin/bash
HOME=/tmp/home
SHELL=/bin/bash
TERM=xterm-256color
USER=builder
PWD=/tmp/work
HISTFILE=/tmp/history
HISTFILESIZE=unlimited
HISTIGNORE='ls:bg:fg:history'
HISTSIZE=-1
HISTTIMEFORMAT='%F %T '
COMP_WORDBREAKS=\"${COMP_WORDBREAKS//:/}\"
PROMPT_COMMAND='history -a'
BASH_ENV=/dev/null
BASH_XTRACEFD=9
ENV=/dev/null
INPUTRC=/tmp/inputrc
MAIL=/tmp/mail
OLDPWD=/tmp/old
PROMPT_DIRTRIM=2
SECONDS=0
TIMEFORMAT='%R'
TMOUT=30
PS1='prompt> '
PS2='continuation> '
PS3=''
PS4='+ '
COLUMNS=1
READLINE_POINT=0
unused=1
";
    let model = model(source);
    model.analysis().dataflow();

    let unused = model
        .analysis()
        .unused_assignments()
        .iter()
        .map(|binding| model.binding(*binding).name.as_str())
        .collect::<Vec<_>>();
    for name in [
        "HOME",
        "SHELL",
        "TERM",
        "USER",
        "PWD",
        "OPTIND",
        "OPTARG",
        "OPTERR",
        "HISTFILE",
        "HISTFILESIZE",
        "HISTIGNORE",
        "HISTSIZE",
        "HISTTIMEFORMAT",
        "COMP_WORDBREAKS",
        "PROMPT_COMMAND",
        "BASH_ENV",
        "BASH_XTRACEFD",
        "ENV",
        "INPUTRC",
        "MAIL",
        "OLDPWD",
        "PROMPT_DIRTRIM",
        "SECONDS",
        "TIMEFORMAT",
        "TMOUT",
        "PS1",
        "PS2",
        "PS3",
        "PS4",
        "COLUMNS",
        "READLINE_POINT",
    ] {
        assert!(!unused.contains(&name), "unused bindings: {:?}", unused);
    }
    assert!(unused.contains(&"unused"));
}

#[test]
fn escaped_ps4_prompt_references_are_read_at_the_assignment_site() {
    let source = "\
#!/bin/bash
export PS4=\"+ \\${BASH_SOURCE##\\${rvm_path:-}} > \"
p=\"$rvm_path\"
";
    let model = model(source);
    let analysis = model.analysis();
    let prompt_reference = analysis
        .uninitialized_references()
        .iter()
        .map(|uninitialized| model.reference(uninitialized.reference))
        .find(|reference| reference.name == "rvm_path" && reference.span.slice(source) == "PS4");

    assert!(prompt_reference.is_some());
}

#[test]
fn trap_action_references_are_read_at_the_action_word() {
    let source = "\
#!/bin/sh
tmpdir=/tmp/example
trap 'ret=$?; rmdir \"$tmpdir/d\" \"$tmpdir\" 2>/dev/null; exit $ret' 0
";
    let model = model(source);
    let analysis = model.analysis();
    let trap_reference = analysis
        .uninitialized_references()
        .iter()
        .map(|uninitialized| model.reference(uninitialized.reference))
        .find(|reference| {
            reference.name == "ret"
                && reference.kind == ReferenceKind::TrapAction
                && reference.span.slice(source)
                    == "'ret=$?; rmdir \"$tmpdir/d\" \"$tmpdir\" 2>/dev/null; exit $ret'"
        });

    assert!(trap_reference.is_some());
}

#[test]
fn bash_completion_runtime_vars_are_treated_as_live() {
    let source = "\
#!/bin/bash
_pyenv() {
  COMPREPLY=()
  local word=\"${COMP_WORDS[COMP_CWORD]}\"
  COMPREPLY=( $(compgen -W \"$(printf 'a b')\" -- \"$word\") )
}
complete -F _pyenv pyenv
";
    let model = model(source);
    model.analysis().dataflow();

    let unused = model
        .analysis()
        .unused_assignments()
        .iter()
        .map(|binding| model.binding(*binding).name.as_str())
        .collect::<Vec<_>>();
    assert!(!unused.contains(&"COMPREPLY"));

    let uninitialized = uninitialized_names(&model);
    assert!(!uninitialized.contains(&"COMP_WORDS".to_string()));
    assert!(!uninitialized.contains(&"COMP_CWORD".to_string()));
}

#[test]
fn exact_indirect_expansion_does_not_keep_unrelated_array_live() {
    let source = "\
#!/bin/bash
apache_args=(--apache)
unused_args=(--unused)
args_var=apache_args[@]
printf '%s\\n' \"${!args_var}\"
";
    let model = model(source);
    model.analysis().dataflow();

    let unused = model
        .analysis()
        .unused_assignments()
        .iter()
        .map(|binding| model.binding(*binding).name.as_str())
        .collect::<Vec<_>>();
    assert!(!unused.contains(&"apache_args"));
    assert!(unused.contains(&"unused_args"));

    let reference = model
        .references()
        .iter()
        .find(|reference| {
            reference.kind == ReferenceKind::IndirectExpansion && reference.name == "args_var"
        })
        .unwrap();
    let targets = binding_names(&model, model.indirect_targets_for_reference(reference.id));
    assert_eq!(targets, vec!["apache_args"]);
}

#[test]
fn exact_indirect_target_resolution_tracks_underlying_binding() {
    let source = "\
#!/bin/bash
target=ok
name=target
printf '%s\\n' \"${!name}\"
";
    let model = model(source);

    let carrier = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "name")
        .unwrap();
    let reference = model
        .references()
        .iter()
        .find(|reference| {
            reference.kind == ReferenceKind::IndirectExpansion && reference.name == "name"
        })
        .unwrap();

    assert_eq!(
        binding_names(&model, model.indirect_targets_for_binding(carrier.id)),
        vec!["target"]
    );
    assert_eq!(
        binding_names(&model, model.indirect_targets_for_reference(reference.id)),
        vec!["target"]
    );
}

#[test]
fn unused_assignment_flags_indirect_only_targets_by_default() {
    let source = "\
#!/bin/bash
target=ok
name=target
other=1
printf '%s\\n' \"${!name}\"
";
    let model = model(source);
    let default_unused = binding_names(&model, model.analysis().unused_assignments());
    let live_indirect_target_unused = binding_names(
        &model,
        model
            .analysis()
            .unused_assignments_with_options(UnusedAssignmentAnalysisOptions {
                treat_indirect_expansion_targets_as_used: true,
                report_unreachable_assignments: false,
            }),
    );

    assert_eq!(default_unused, vec!["target", "other"]);
    assert_eq!(live_indirect_target_unused, vec!["other"]);
}

#[test]
fn unused_assignment_options_keep_array_like_indirect_targets_live() {
    let source = "\
#!/bin/bash
apache_args=(--apache)
unused_args=(--unused)
args_var=apache_args[@]
printf '%s\\n' \"${!args_var}\"
";
    let model = model(source);
    let default_unused = binding_names(&model, model.analysis().unused_assignments());
    let compat_unused = binding_names(
        &model,
        model
            .analysis()
            .unused_assignments_with_options(UnusedAssignmentAnalysisOptions {
                treat_indirect_expansion_targets_as_used: false,
                report_unreachable_assignments: false,
            }),
    );

    assert_eq!(default_unused, vec!["unused_args"]);
    assert_eq!(compat_unused, vec!["unused_args"]);
}

#[test]
fn unused_assignment_options_preserve_heuristic_fast_path_without_indirect_targets() {
    let model = model("unused=1\n");
    let analysis = model.analysis();

    let compat_unused = analysis
        .unused_assignments_with_options(UnusedAssignmentAnalysisOptions {
            treat_indirect_expansion_targets_as_used: false,
            report_unreachable_assignments: false,
        })
        .to_vec();

    assert!(analysis.exact_variable_dataflow.get().is_none());
    assert!(analysis.dataflow.get().is_none());
    assert_eq!(binding_names(&model, &compat_unused), vec!["unused"]);
}

#[test]
fn unused_assignment_options_can_report_unreachable_assignments() {
    let source = "\
#!/bin/bash
f() {
  return 1
  dead=1
  used=1
  printf '%s\\n' \"$used\"
}
f
";
    let model = model(source);
    let default_unused = binding_names(&model, model.analysis().unused_assignments());
    let compat_unused = binding_names(
        &model,
        model
            .analysis()
            .unused_assignments_with_options(UnusedAssignmentAnalysisOptions {
                treat_indirect_expansion_targets_as_used: false,
                report_unreachable_assignments: true,
            }),
    );

    assert!(!default_unused.iter().any(|name| name == "dead"));
    assert_eq!(compat_unused, vec!["dead"]);
}

#[test]
fn resolved_indirect_expansion_carrier_is_not_marked_uninitialized() {
    let source = "\
#!/bin/bash
f() {
  local carrier
  echo \"${!carrier}\"
}
f
";
    let model = model(source);
    assert!(uninitialized_names(&model).is_empty());
}

#[test]
fn prefix_name_expansions_do_not_read_the_prefix_as_a_variable() {
    let source = "unset \"${!completion_prefix@}\"\nprintf '%s\\n' \"$ordinary_missing\"\n";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["completion_prefix"], &uninitialized);
    assert_names_present(&["ordinary_missing"], &uninitialized);
}

#[test]
fn guarded_parameter_expansions_are_not_marked_uninitialized() {
    let source = "\
printf '%s\\n' \
  \"${missing_default:-fallback}\" \
  \"${missing_assign:=value}\" \
  \"${missing_replace:+alt}\" \
  \"${missing_error:?missing}\"
";
    let model = model(source);
    let unresolved = unresolved_names(&model);
    let uninitialized = uninitialized_names(&model);

    assert_names_present(
        &[
            "missing_default",
            "missing_assign",
            "missing_replace",
            "missing_error",
        ],
        &unresolved,
    );
    assert_names_absent(
        &[
            "missing_default",
            "missing_assign",
            "missing_replace",
            "missing_error",
        ],
        &uninitialized,
    );
}

#[test]
fn unquoted_guarded_parameter_expansions_are_not_marked_uninitialized() {
    let source = "\
eval start-stop-daemon --start \\
  ${directory:+--chdir} $directory
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["directory"], &uninitialized);
}

#[test]
fn self_referential_assignments_are_not_marked_uninitialized() {
    let source = "\
foo=\"$foo\"
for flag in a b; do
  valid_flags=\"${valid_flags} $flag\"
done
foo[$foo]=x
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["foo", "valid_flags"], &uninitialized);
}

#[test]
fn escaped_declaration_builtins_initialize_dynamic_assignment_operands() {
    let source = "\
\\typeset ret=$?
printf '%s\\n' \"$ret\"
";
    let model = model_with_dialect(source, ShellDialect::Bash);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["ret"], &uninitialized);
}

#[test]
fn arithmetic_conditional_literal_operands_are_uninitialized_reads() {
    let source = "\
version=1
if [[ $version -eq \"latest\" ]]; then
  :
fi
if [[ 1 -ne bare ]]; then
  :
fi
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_present(&["latest", "bare"], &uninitialized);
}

#[test]
fn let_arithmetic_assignments_initialize_targets() {
    let source = "\
#!/bin/bash
let line=\"$number\"+1
printf '%s\\n' \"$line\"
";
    let model = model_with_dialect(source, ShellDialect::Bash);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["line"], &uninitialized);
    assert_names_present(&["number"], &uninitialized);
}

#[test]
fn assignment_values_continue_after_escaped_newlines() {
    let source = "\
#!/bin/sh
easyrsa_ksh=\\
'value'
[ \"${KSH_VERSION}\" = \"${easyrsa_ksh}\" ] && echo ok
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["easyrsa_ksh"], &uninitialized);
}

#[test]
fn special_zero_prefix_removal_inside_escaped_quotes_does_not_synthesize_empty_references() {
    let source = "\
#!/bin/bash
usage=\"
Terraform:

    data \\\"external\\\" \\\"github_repos\\\" {
        program = [\\\"/path/to/${0##*/}\\\", \\\"github_repository\\\"]
    }
\"
";
    let model = model_with_dialect(source, ShellDialect::Bash);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&[""], &uninitialized);
}

#[test]
fn unparsed_indexed_subscript_prefixes_are_uninitialized_reads() {
    let source = "\
arr+=([docker:dind]=x [nats-streaming:nanoserver]=y)
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_present(&["docker", "nats", "streaming"], &uninitialized);
    assert_names_absent(&["dind", "nanoserver"], &uninitialized);
}

#[test]
fn escaped_heredoc_parameter_literals_still_expand_nested_references() {
    let source = "\
cat <<EOF
\\${OUTER:-$inner}
EOF
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_present(&["inner"], &uninitialized);
    assert_names_absent(&["OUTER"], &uninitialized);
}

#[test]
fn assign_default_and_error_operands_are_marked_uninitialized() {
    let source = "\
printf '%s\\n' \
  \"${missing_default:-$fallback_name}\" \
  \"${missing_assign:=$seed_name}\" \
  \"${missing_replace:+$replacement_name}\" \
  \"${missing_error:?$hint_name}\"
";
    let model = model(source);
    let unresolved = unresolved_names(&model);
    let uninitialized = uninitialized_names(&model);

    assert_names_present(
        &[
            "fallback_name",
            "seed_name",
            "replacement_name",
            "hint_name",
        ],
        &unresolved,
    );
    assert_names_absent(&["fallback_name", "replacement_name"], &uninitialized);
    assert_names_present(&["seed_name", "hint_name"], &uninitialized);
}

#[test]
fn parameter_slice_arithmetic_operands_are_not_uninitialized() {
    let source = "\
value=abcdef
printf '%s\\n' \"${value:offset}\" \"${value:1:$length}\"
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["offset", "length"], &uninitialized);
    assert!(model.references().iter().any(|reference| {
        reference.kind == ReferenceKind::ParameterSliceArithmetic && reference.name == "offset"
    }));
    assert!(model.references().iter().any(|reference| {
        reference.kind == ReferenceKind::ParameterSliceArithmetic && reference.name == "length"
    }));
}

#[test]
fn defaulting_parameter_operand_references_are_marked_for_sc2154_suppression() {
    let source = "\
printf '%s\\n' \
  \"${missing_default:-$fallback_name}\" \
  \"${missing_assign:=$seed_name}\" \
  \"${missing_replace:+$replacement_name}\" \
  \"${missing_error:?$hint_name}\"
";
    let model = model(source);
    let suppressed = model
        .references()
        .iter()
        .filter(|reference| model.is_defaulting_parameter_operand_reference(reference.id))
        .map(|reference| reference.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        suppressed,
        vec![
            "fallback_name",
            "seed_name",
            "replacement_name",
            "hint_name",
        ]
    );
}

#[test]
fn guarded_or_defaulting_reference_offsets_are_grouped_by_name() {
    let source = "\
printf '%s\\n' \
  \"${missing_default:-$fallback_name}\" \
  \"${missing_assign:=$seed_name}\" \
  \"${missing_replace:+$seed_name}\" \
  \"${missing_error:?$hint_name}\" \
  \"${missing_error:?$hint_name}\"
";
    let model = model(source);
    let mut expected = FxHashMap::<String, Vec<usize>>::default();

    for reference in model.references() {
        if model.is_guarded_parameter_reference(reference.id)
            || model.is_defaulting_parameter_operand_reference(reference.id)
        {
            expected
                .entry(reference.name.as_str().to_owned())
                .or_default()
                .push(reference.span.start.offset);
        }
    }

    for offsets in expected.values_mut() {
        offsets.sort_unstable();
        offsets.dedup();
    }

    let actual = model
        .guarded_or_defaulting_reference_offsets_by_name()
        .iter()
        .map(|(name, offsets)| (name.as_str().to_owned(), offsets.to_vec()))
        .collect::<FxHashMap<_, _>>();

    assert_eq!(actual, expected);
}

#[test]
fn function_positional_reference_summary_respects_local_transient_resets() {
    let source = "\
greet() {
  (
    set -- inner
    printf '%s\\n' \"$1\"
  )
}
relay() {
  printf '%s\\n' \"$@\"
}
";
    let model = model(source);

    let greet_use_offset = span_for_nth(source, "printf '%s\\n' \"$1\"", 0)
        .start
        .offset;
    let relay_use_offset = span_for_nth(source, "printf '%s\\n' \"$@\"", 0)
        .start
        .offset;
    let greet_scope = model
        .enclosing_function_scope(model.scope_at(greet_use_offset))
        .unwrap();
    let relay_scope = model
        .enclosing_function_scope(model.scope_at(relay_use_offset))
        .unwrap();

    let without_resets = model.function_positional_reference_summary(&FxHashMap::default());
    assert_eq!(
        without_resets
            .get(&greet_scope)
            .copied()
            .unwrap()
            .required_arg_count(),
        1
    );
    assert!(
        without_resets
            .get(&greet_scope)
            .copied()
            .unwrap()
            .uses_unprotected_positional_parameters()
    );
    assert_eq!(
        without_resets
            .get(&relay_scope)
            .copied()
            .unwrap()
            .required_arg_count(),
        0
    );
    assert!(
        without_resets
            .get(&relay_scope)
            .copied()
            .unwrap()
            .uses_unprotected_positional_parameters()
    );

    let transient_scope = model
        .innermost_transient_scope_within_function(model.scope_at(greet_use_offset))
        .unwrap();
    let reset_offset = span_for_nth(source, "set -- inner", 0).start.offset;
    let mut local_resets = FxHashMap::default();
    local_resets.insert(transient_scope, vec![reset_offset]);

    let with_resets = model.function_positional_reference_summary(&local_resets);
    assert!(!with_resets.contains_key(&greet_scope));
    assert_eq!(
        with_resets
            .get(&relay_scope)
            .copied()
            .unwrap()
            .required_arg_count(),
        0
    );
    assert!(
        with_resets
            .get(&relay_scope)
            .copied()
            .unwrap()
            .uses_unprotected_positional_parameters()
    );
}

#[test]
fn branch_initialized_names_stay_initialized_inside_command_substitutions() {
    let source = "\
if [ \"$1\" = h ]; then
  humanreadable=-h
else
  humanreadable=-m
fi
value=\"$(free ${humanreadable} | awk '{print $2}')\"
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["humanreadable"], &uninitialized);
}

#[test]
fn assign_default_parameter_expansion_initializes_later_reads() {
    let source = "\
printf '%s\\n' \"${config_path:=/tmp/default}\"
printf '%s\\n' \"$config_path\" \"$still_missing\"
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["config_path"], &uninitialized);
    assert_names_present(&["still_missing"], &uninitialized);

    let binding = model
        .bindings()
        .iter()
        .find(|binding| {
            binding.name == "config_path"
                && matches!(binding.kind, BindingKind::ParameterDefaultAssignment)
        })
        .unwrap();
    assert_eq!(binding.span.slice(source), "${config_path:=/tmp/default}");
}

#[test]
fn parameter_guard_flow_initializes_later_c006_reads() {
    let source = "\
printf '%s\\n' \"${defaulted:-fallback}\"
printf '%s\\n' \"${assigned:=fallback}\"
printf '%s\\n' \"${required:?missing}\"
printf '%s\\n' \"${replacement:+alt}\"
printf '%s\\n' \"$defaulted\" \"$assigned\" \"$required\" \"$replacement\" \"$still_missing\"
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(
        &["defaulted", "assigned", "required", "replacement"],
        &uninitialized,
    );
    assert_names_present(&["still_missing"], &uninitialized);
}

#[test]
fn parameter_guard_flow_respects_same_command_order() {
    let source = "\
printf '%s\\n' \
  \"${same_default:-fallback}\" \"$same_default\" \
  \"${same_assigned:=fallback}\" \"$same_assigned\" \
  \"${same_required:?missing}\" \"$same_required\" \
  \"${same_replacement:+alt}\" \"$same_replacement\" \
  \"$before_default\" \"${before_default:-fallback}\" \
  \"$still_missing\"
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(
        &[
            "same_default",
            "same_assigned",
            "same_required",
            "same_replacement",
        ],
        &uninitialized,
    );
    assert_names_present(&["before_default", "still_missing"], &uninitialized);
}

#[test]
fn parameter_guard_flow_does_not_escape_conditional_operands() {
    let source = "\
printf '%s\\n' \"${outer:+${nested_default:-fallback}}\" \"$outer\" \"$nested_default\"
printf '%s\\n' \"${other:+${nested_replacement:+alt}}\" \"$other\" \"$nested_replacement\"
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["outer", "other"], &uninitialized);
    assert_names_present(&["nested_default", "nested_replacement"], &uninitialized);
}

#[test]
fn parameter_guard_flow_does_not_escape_short_circuit_conditionals() {
    let source = "\
flag=
[[ ${left_guard:-fallback} && $flag ]]
printf '%s\\n' \"$left_guard\"
[[ $flag && ${right_and:-fallback} ]]
printf '%s\\n' \"$right_and\"
flag=1
[[ $flag || ${right_or:+alt} ]]
printf '%s\\n' \"$right_or\"
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&["left_guard"], &uninitialized);
    assert_names_present(&["right_and", "right_or"], &uninitialized);
}

#[test]
fn non_assigning_parameter_guard_flow_does_not_create_bindings() {
    let source = "\
: \"${defaulted:-fallback}\" \"${replacement:+alt}\"
";
    let model = model(source);

    assert!(model.bindings_for(&Name::from("defaulted")).is_empty());
    assert!(model.bindings_for(&Name::from("replacement")).is_empty());
    assert!(
        model
            .analysis()
            .summarize_scope_provided_bindings(ScopeId(0))
            .is_empty()
    );
}

#[test]
fn parameter_reference_spans_exclude_escaped_quotes_in_double_quoted_strings() {
    let source = "\
#!/bin/bash
rvm_info=\"
  uname: \\\"${_system_info}\\\"
\"
";
    let model = model(source);
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "_system_info")
        .unwrap();

    assert_eq!(reference.span.start.line, 3);
    assert_eq!(reference.span.start.column, 12);
    assert_eq!(reference.span.slice(source), "${_system_info}");
}

#[test]
fn parameter_reference_spans_recover_after_escaped_quotes_and_tabs_in_assignments() {
    let source = "\
#!/bin/bash
physmemtotal=\"${physmemtotal//,/.}\"
payload=\"{
\t\\\"client_id\\\": \\\"${uuidinstance}\\\",
\t\\\"events\\\": [
\t\t{
\t\t\\\"name\\\": \\\"LinuxGSM\\\",
\t\t\\\"params\\\": {
\t\t\t\\\"cpuusedmhzroundup\\\": \\\"${cpuusedmhzroundup}\\\",
\t\t\t\\\"diskused\\\": \\\"${serverfilesdu}\\\",
\t\t\t}
\t\t}
\t]
}\"
";
    let model = model(source);
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "serverfilesdu")
        .unwrap();

    assert_eq!(reference.span.start.line, 10);
    assert_eq!(reference.span.start.column, 20);
    assert_eq!(reference.span.slice(source), "${serverfilesdu}");
}

#[test]
fn unbraced_parameter_reference_spans_recover_after_escaped_quotes() {
    let source = "\
#!/bin/bash
rvm_info=\"
  path:         \\\"$rvm_path\\\"
\"
addtimestamp=\"gawk '{ print strftime(\\\\\\\"[$logtimestampformat]\\\\\\\"), \\\\\\$0 }'\"
";
    let model = model(source);
    let rvm_path = model
        .references()
        .iter()
        .find(|reference| reference.name == "rvm_path")
        .unwrap();
    let logtimestampformat = model
        .references()
        .iter()
        .find(|reference| reference.name == "logtimestampformat")
        .unwrap();

    assert_eq!(rvm_path.span.slice(source), "$rvm_path");
    assert_eq!(logtimestampformat.span.slice(source), "$logtimestampformat");
}

#[test]
fn parameter_reference_spans_include_nested_parameter_operator_suffixes() {
    let source = "\
rvm_ruby_gem_home=\"${rvm_ruby_gem_home%%${rvm_gemset_separator:-\"@\"}*}\"
if [ \"${skiprdeps/${_lf}/}\" != \"${skiprdeps}\" ]; then
  :
fi
";
    let model = model(source);
    let rvm_gem_home = model
        .references()
        .iter()
        .find(|reference| reference.name == "rvm_ruby_gem_home")
        .unwrap();
    let skiprdeps = model
        .references()
        .iter()
        .find(|reference| reference.name == "skiprdeps")
        .unwrap();

    assert_eq!(
        rvm_gem_home.span.slice(source),
        "${rvm_ruby_gem_home%%${rvm_gemset_separator:-\"@\"}*}"
    );
    assert_eq!(rvm_gem_home.span.end.column, 71);
    assert_eq!(skiprdeps.span.slice(source), "${skiprdeps/${_lf}/}");
    assert_eq!(skiprdeps.span.end.column, 27);
}

#[test]
fn default_parameter_operand_reads_are_tracked() {
    let source = "\
repo_root=$(pwd)
cache_dir=${1:-\"$repo_root/.cache\"}
printf '%s\\n' \"$cache_dir\"
";
    let model = model_with_dialect(source, ShellDialect::Posix);
    let unused = reportable_unused_names(&model);

    assert!(
        !unused.contains(&Name::from("repo_root")),
        "unused bindings: {:?}",
        unused
    );

    let reference = model
        .references()
        .iter()
        .find(|reference| {
            reference.kind == ReferenceKind::Expansion && reference.name == "repo_root"
        })
        .unwrap();
    let binding = model.resolved_binding(reference.id).unwrap();
    assert_eq!(binding.name, "repo_root");
}

#[test]
fn self_referential_default_initializers_are_not_reported_unused() {
    let source = "\
#!/bin/sh
STATE=\"${STATE:-in_progress}\"
DESCRIPTION=\"${DESCRIPTION:-Deployment metadata updated}\"
";
    let model = model_with_dialect(source, ShellDialect::Posix);
    let unused = reportable_unused_names(&model);

    assert!(
        !unused.contains(&Name::from("STATE")),
        "unused bindings: {:?}",
        unused
    );
    assert!(
        !unused.contains(&Name::from("DESCRIPTION")),
        "unused bindings: {:?}",
        unused
    );
}

#[test]
fn detects_dead_code_after_exit() {
    let source = "exit 0\necho dead\n";
    let model = model(source);
    let analysis = model.analysis();
    let dead_code = analysis.dead_code();
    assert_eq!(dead_code.len(), 1);
    assert_eq!(
        dead_code[0].unreachable[0].slice(source).trim_end(),
        "echo dead"
    );
    assert_eq!(dead_code[0].cause.slice(source).trim_end(), "exit 0");
}

#[test]
fn loop_control_condition_keeps_unreachable_if_tree_causes() {
    let source = "\
while true; do
  if break; then
    printf '%s\\n' after_true
  else
    printf '%s\\n' after_false
  fi
  printf '%s\\n' after_if
done
";
    let model = model(source);
    let analysis = model.analysis();
    let dead_code = analysis.dead_code();
    let unreachable = dead_code
        .iter()
        .flat_map(|entry| entry.unreachable.iter())
        .map(|span| span.slice(source).trim_end().to_owned())
        .collect::<Vec<_>>();

    assert!(unreachable.contains(&"printf '%s\\n' after_true".to_owned()));
    assert!(unreachable.contains(&"printf '%s\\n' after_false".to_owned()));
    assert!(unreachable.contains(&"printf '%s\\n' after_if".to_owned()));
    assert!(
        dead_code
            .iter()
            .all(|entry| entry.cause_kind == UnreachableCauseKind::LoopControl),
        "dead code: {dead_code:?}"
    );
}

#[test]
fn compound_dead_code_reports_outermost_spans() {
    let source = "\
return
if true; then
  echo one
fi
printf '%s\\n' two
";
    let model = model(source);
    let analysis = model.analysis();
    let dead_code = analysis.dead_code();
    let unreachable = dead_code
        .iter()
        .flat_map(|entry| entry.unreachable.iter())
        .map(|span| span.slice(source).trim_end().to_owned())
        .collect::<Vec<_>>();

    assert!(unreachable.contains(&"if true; then\n  echo one\nfi".to_owned()));
    assert!(unreachable.contains(&"printf '%s\\n' two".to_owned()));
    assert!(!unreachable.contains(&"echo one".to_owned()));
}

#[test]
fn dead_code_after_script_terminating_function_calls_is_detected() {
    let source = "\
exit_script() {
  exit 0
}
main() {
  exit_script
  printf '%s\\n' never
}
";
    let model = model(source);
    let analysis = model.analysis();
    let unreachable = analysis
        .dead_code()
        .iter()
        .flat_map(|entry| entry.unreachable.iter())
        .map(|span| span.slice(source).trim_end().to_owned())
        .collect::<Vec<_>>();

    assert!(unreachable.contains(&"printf '%s\\n' never".to_owned()));
}

#[test]
fn resolved_function_calls_feed_dataflow_and_script_termination() {
    let source = "\
reader() {
  printf '%s\\n' \"$value\"
}
exit_script() {
  exit 0
}
main() {
  value=ok
  reader
  exit_script
  printf '%s\\n' never
}
main
";
    let model = model(source);
    let analysis = model.analysis();
    let unused = binding_names(&model, analysis.dataflow().unused_assignment_ids());
    let unreachable = analysis
        .dead_code()
        .iter()
        .flat_map(|entry| entry.unreachable.iter())
        .map(|span| span.slice(source).trim_end().to_owned())
        .collect::<Vec<_>>();

    assert!(
        !unused.iter().any(|name| name == "value"),
        "unused bindings: {unused:?}"
    );
    assert!(
        unreachable.contains(&"printf '%s\\n' never".to_owned()),
        "unreachable spans: {unreachable:?}"
    );
}

#[test]
fn condition_body_after_script_terminating_condition_is_dead() {
    let source = "\
if exit 0; then
  printf '%s\\n' never
fi
";
    let model = model(source);
    let unreachable = model
        .analysis()
        .dead_code()
        .iter()
        .flat_map(|entry| entry.unreachable.iter())
        .map(|span| span.slice(source).trim_end().to_owned())
        .collect::<Vec<_>>();

    assert!(
        unreachable.contains(&"printf '%s\\n' never".to_owned()),
        "unreachable spans: {unreachable:?}"
    );
}

#[test]
fn case_return_paths_keep_helper_from_being_script_terminating() {
    let source = "\
die() {
  exit 1
}
helper() {
  case \"$1\" in
    ok)
      return
      ;;
  esac
  die
}
main() {
  helper ok
  printf '%s\\n' still_reachable
}
";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn loop_return_paths_keep_helper_from_being_script_terminating() {
    let source = "\
die() {
  exit 1
}
helper() {
  for item in 1; do
    return
  done
  die
}
main() {
  helper
  printf '%s\\n' still_reachable
}
";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn script_terminating_calls_use_rewritten_statement_spans() {
    let sources = [
        "\
exit_script() {
  exit 0
}
main() {
  exit_script >/dev/null
  printf '%s\\n' never
}
",
        "\
exit_script() {
  exit 0
}
main() {
  ! exit_script
  printf '%s\\n' never
}
",
    ];

    for source in sources {
        let model = model(source);
        let analysis = model.analysis();
        let unreachable = analysis
            .dead_code()
            .iter()
            .flat_map(|entry| entry.unreachable.iter())
            .map(|span| span.slice(source).trim_end().to_owned())
            .collect::<Vec<_>>();

        assert!(
            unreachable.contains(&"printf '%s\\n' never".to_owned()),
            "unreachable spans: {unreachable:?}"
        );
    }
}

#[test]
fn sourceable_file_return_keeps_helper_exit_calls_non_terminating() {
    let source = "\
[ -n \"$loaded\" ] && return
loaded=1
exit_script() {
  exit 0
}
main() {
  exit_script
  printf '%s\\n' still_reachable
}
";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn sourceable_file_return_before_helper_keeps_helper_exit_calls_non_terminating() {
    let source = "\
already_loaded() {
  :
}
[ -n \"$loaded\" ] && return
loaded=1
exit_script() {
  exit 0
}
main() {
  exit_script
  printf '%s\\n' still_reachable
}
";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn sourceable_file_return_after_helper_keeps_helper_exit_calls_terminating() {
    let source = "\
main() {
  exit_script
  printf '%s\\n' never
}
{
  exit_script() {
    exit 0
  }
  return
}
";
    let model = model(source);
    let unreachable = model
        .analysis()
        .dead_code()
        .iter()
        .flat_map(|dead_code| dead_code.unreachable.iter())
        .map(|span| span.slice(source).to_owned())
        .collect::<Vec<_>>();

    assert!(
        unreachable.contains(&"printf '%s\\n' never\n".to_owned()),
        "unreachable spans: {unreachable:?}"
    );
}

#[test]
fn brace_group_function_definitions_can_make_later_calls_terminating() {
    let source = "\
{
  exit_script() {
    exit 0
  }
}
main() {
  exit_script
  printf '%s\\n' never
}
";
    let model = model(source);
    let analysis = model.analysis();
    let unreachable = analysis
        .dead_code()
        .iter()
        .flat_map(|entry| entry.unreachable.iter())
        .map(|span| span.slice(source).trim_end().to_owned())
        .collect::<Vec<_>>();

    assert!(unreachable.contains(&"printf '%s\\n' never".to_owned()));
}

#[test]
fn later_parent_scope_function_definitions_can_terminate_later_runtime_calls() {
    let source = "\
main() {
  exit_script
  printf '%s\\n' never
}
exit_script() {
  exit 0
}
main
";
    let model = model(source);
    let analysis = model.analysis();
    let unreachable = analysis
        .dead_code()
        .iter()
        .flat_map(|entry| entry.unreachable.iter())
        .map(|span| span.slice(source).trim_end().to_owned())
        .collect::<Vec<_>>();

    assert!(unreachable.contains(&"printf '%s\\n' never".to_owned()));
}

#[test]
fn later_function_definitions_do_not_make_earlier_calls_terminating() {
    let source = "\
main() {
  exit_script
  printf '%s\\n' still_reachable
}
main
exit_script() {
  exit 0
}
";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn transitive_calls_before_parent_definitions_keep_later_code_reachable() {
    let source = "\
main() {
  helper
}
helper() {
  inner
}
inner() {
  exit_script
  printf '%s\\n' maybe
}
if should_run; then
  main
fi
exit_script() {
  exit 0
}
main
";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn top_level_parent_call_before_nested_definition_keeps_later_code_reachable() {
    let source = "\
outer() {
  inner() {
    helper
    printf '%s\\n' maybe
  }
  inner
  helper() {
    exit 0
  }
}
outer
";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn nested_calls_after_parent_definition_can_use_script_terminating_helpers() {
    let source = "\
outer() {
  inner() {
    helper
    printf '%s\\n' never
  }
  helper() {
    exit 0
  }
  inner
}
outer
";
    let model = model(source);
    let unreachable = model
        .analysis()
        .dead_code()
        .iter()
        .flat_map(|entry| entry.unreachable.iter())
        .map(|span| span.slice(source).trim_end().to_owned())
        .collect::<Vec<_>>();

    assert!(
        unreachable.contains(&"printf '%s\\n' never".to_owned()),
        "unreachable spans: {unreachable:?}"
    );
}

#[test]
fn later_redefinitions_do_not_fall_back_to_stale_terminating_helpers() {
    let source = "\
exit_script() {
  exit 0
}
main() {
  exit_script
  printf '%s\\n' maybe
}
if should_run; then
  main
fi
exit_script() {
  :
}
main
";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn conditional_function_definitions_do_not_make_calls_terminating() {
    let source = "\
if false; then
  exit_script() {
    exit 0
  }
fi
exit_script
printf '%s\\n' still_reachable
";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn conditional_exit_keeps_or_fallback_reachable() {
    let source = "run && exit 0 || echo fallback\n";
    let model = model(source);

    assert!(
        model.analysis().dead_code().is_empty(),
        "dead code: {:?}",
        model.analysis().dead_code()
    );
}

#[test]
fn deferred_function_bodies_resolve_later_file_scope_bindings() {
    let source = "f() { echo $X; }\nX=1\nf\n";
    let model = model(source);

    let reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "X")
        .unwrap();
    let binding = model.resolved_binding(reference.id).unwrap();
    assert_eq!(binding.span.slice(source), "X");
}

#[test]
fn deferred_non_brace_function_bodies_resolve_later_file_scope_bindings() {
    let source = "f() if true; then echo $X; fi\nX=1\nf\n";
    let model = model(source);

    let reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "X")
        .unwrap();
    let binding = model.resolved_binding(reference.id).unwrap();
    assert_eq!(binding.span.slice(source), "X");
}

#[test]
fn top_level_reads_remain_source_order_sensitive() {
    let source = "echo $X\nX=1\n";
    let model = model(source);

    let reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "X")
        .unwrap();
    assert!(model.resolved_binding(reference.id).is_none());
    assert_eq!(model.unresolved_references(), &[reference.id]);
}

#[test]
fn common_runtime_vars_are_not_marked_uninitialized_in_bash_and_sh_scripts() {
    let names = [
        "IFS",
        "USER",
        "HOME",
        "SHELL",
        "PWD",
        "TERM",
        "PATH",
        "CDPATH",
        "LANG",
        "LC_ALL",
        "LC_TIME",
        "SUDO_USER",
        "DOAS_USER",
    ];

    for shebang in ["#!/bin/bash", "#!/bin/sh"] {
        let source = common_runtime_source(shebang);
        let model = model(&source);
        let unresolved = unresolved_names(&model);
        let uninitialized = uninitialized_names(&model);

        assert_names_absent(&names, &unresolved);
        assert_names_absent(&names, &uninitialized);
    }
}

#[test]
fn bash_runtime_vars_are_not_marked_uninitialized_in_bash_scripts() {
    let source = bash_runtime_source("#!/bin/bash");
    let model = model(&source);
    let names = [
        "LINENO",
        "FUNCNAME",
        "BASH_SOURCE",
        "BASH_LINENO",
        "RANDOM",
        "BASH_REMATCH",
        "READLINE_LINE",
        "BASH_VERSION",
        "BASH_VERSINFO",
        "OSTYPE",
        "HISTCONTROL",
        "HISTSIZE",
    ];

    let unresolved = unresolved_names(&model);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&names, &unresolved);
    assert_names_absent(&names, &uninitialized);
}

#[test]
fn env_split_bash_shebang_enables_bash_runtime_vars() {
    let source = bash_runtime_source("#!/usr/bin/env -S bash -e");
    let model = model(&source);
    let names = [
        "LINENO",
        "FUNCNAME",
        "BASH_SOURCE",
        "BASH_LINENO",
        "RANDOM",
        "BASH_REMATCH",
        "READLINE_LINE",
        "BASH_VERSION",
        "BASH_VERSINFO",
        "OSTYPE",
        "HISTCONTROL",
        "HISTSIZE",
    ];

    let unresolved = unresolved_names(&model);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&names, &unresolved);
    assert_names_absent(&names, &uninitialized);
}

#[test]
fn inferred_profile_honors_env_split_shebang() {
    assert_eq!(
        infer_parse_dialect_from_source("#!/usr/bin/env -S sh -e\n:\n", None),
        ShellDialect::Posix
    );
    assert_eq!(
        infer_parse_dialect_from_source("#!/usr/bin/env -S zsh -f\nprint ok\n", None),
        ShellDialect::Zsh
    );
}

#[test]
fn bash_runtime_array_references_are_classified() {
    let source = "#!/bin/bash\nprintf '%s\\n' \"$BASH_SOURCE\" \"$FUNCNAME\" \"$RANDOM\"\n";
    let model = model(source);

    for name in ["BASH_SOURCE", "FUNCNAME"] {
        let reference = model
            .references()
            .iter()
            .find(|reference| reference.name == name)
            .unwrap();
        assert!(model.reference_is_predefined_runtime_array(reference.id));
    }

    let random = model
        .references()
        .iter()
        .find(|reference| reference.name == "RANDOM")
        .unwrap();
    assert!(!model.reference_is_predefined_runtime_array(random.id));
}

#[test]
fn zsh_runtime_vars_are_not_marked_uninitialized_in_zsh_scripts() {
    let source = zsh_runtime_source("#!/bin/zsh");
    let model = model(&source);
    let names = [
        "options",
        "functions",
        "aliases",
        "commands",
        "parameters",
        "termcap",
        "terminfo",
        "path",
        "pipestatus",
        "funcstack",
        "funcfiletrace",
        "funcsourcetrace",
        "psvar",
        "widgets",
        "zsh_eval_context",
        "module_path",
        "manpath",
        "mailpath",
        "historywords",
        "jobdirs",
        "jobstates",
        "jobtexts",
        "signals",
        "MATCH",
        "MBEGIN",
        "MEND",
        "BUFFER",
        "LBUFFER",
        "RBUFFER",
        "CURSOR",
        "WIDGET",
        "KEYS",
        "NUMERIC",
        "POSTDISPLAY",
        "region_highlight",
        "LINES",
        "COLUMNS",
        "ZSH_VERSION",
        "ZSH_NAME",
        "ZSH_PATCHLEVEL",
        "ZSH_SUBSHELL",
        "ZSH_ARGZERO",
    ];

    let unresolved = unresolved_names(&model);
    let uninitialized = uninitialized_names(&model);

    assert_names_absent(&names, &unresolved);
    assert_names_absent(&names, &uninitialized);
}

#[test]
fn zsh_runtime_array_references_are_classified() {
    let source = "\
#!/bin/zsh
printf '%s\\n' \"${path[1]}\" \"${options[xtrace]}\" \"${pipestatus[1]}\" \"$ZSH_VERSION\"
";
    let model = model(source);

    for name in ["path", "options", "pipestatus"] {
        let reference = model
            .references()
            .iter()
            .find(|reference| reference.name == name)
            .unwrap();
        assert!(model.reference_is_predefined_runtime_array(reference.id));
    }

    let version = model
        .references()
        .iter()
        .find(|reference| reference.name == "ZSH_VERSION")
        .unwrap();
    assert!(!model.reference_is_predefined_runtime_array(version.id));
}

#[test]
fn bash_runtime_vars_remain_unresolved_in_non_bash_scripts() {
    let source = bash_runtime_source("#!/bin/sh");
    let model = model(&source);
    let names = [
        "LINENO",
        "FUNCNAME",
        "BASH_SOURCE",
        "BASH_LINENO",
        "RANDOM",
        "BASH_REMATCH",
        "READLINE_LINE",
        "BASH_VERSION",
        "BASH_VERSINFO",
        "OSTYPE",
        "HISTCONTROL",
        "HISTSIZE",
    ];

    let unresolved = unresolved_names(&model);
    let uninitialized = uninitialized_names(&model);

    assert_names_present(&names, &unresolved);
    assert_names_present(&names, &uninitialized);
}

#[test]
fn deferred_nested_function_bodies_resolve_later_outer_bindings() {
    let source = "\
outer() {
  inner() { echo $X; }
  X=1
  inner
}
outer
";
    let model = model(source);

    let reference = model
        .references()
        .iter()
        .find(|reference| reference.kind == ReferenceKind::Expansion && reference.name == "X")
        .unwrap();
    let binding = model.resolved_binding(reference.id).unwrap();
    assert_eq!(binding.span.slice(source).trim(), "X");
    assert!(matches!(
        model.scope_kind(binding.scope),
        ScopeKind::Function(function) if function.contains_name_str("outer")
    ));
}

#[test]
fn top_level_assignment_read_by_later_function_call_is_live() {
    let source = "\
show() { echo \"$flag\"; }
flag=1
show
";
    let model = model(source);

    let unused = reportable_unused_names(&model);
    assert!(unused.is_empty(), "unused: {:?}", unused);
}

#[test]
fn sourced_helper_reads_keep_top_level_assignment_live() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
flag=1
. ./helper.sh
",
    )
    .unwrap();
    fs::write(&helper, "echo \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
}

#[test]
fn bash_source_file_suffix_reads_keep_top_level_assignment_live_transitively() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.bash");
    let loader = temp.path().join("loader.bash");
    let helper = temp.path().join("loader.bash__dep.bash");
    fs::write(
        &main,
        "\
#!/bin/bash
flag=1
source ./loader.bash
",
    )
    .unwrap();
    fs::write(
        &loader,
        "\
#!/bin/bash
source \"${BASH_SOURCE[0]}__dep.bash\"
",
    )
    .unwrap();
    fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
}

#[test]
fn bash_source_double_zero_suffix_reads_keep_top_level_assignment_live_transitively() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.bash");
    let loader = temp.path().join("loader.bash");
    let helper = temp.path().join("loader.bash__dep.bash");
    fs::write(
        &main,
        "\
#!/bin/bash
flag=1
source ./loader.bash
",
    )
    .unwrap();
    fs::write(
        &loader,
        "\
#!/bin/bash
source \"${BASH_SOURCE[00]}__dep.bash\"
",
    )
    .unwrap();
    fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
}

#[test]
fn bash_source_spaced_zero_suffix_reads_keep_top_level_assignment_live_transitively() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.bash");
    let loader = temp.path().join("loader.bash");
    let helper = temp.path().join("loader.bash__dep.bash");
    fs::write(
        &main,
        "\
#!/bin/bash
flag=1
source ./loader.bash
",
    )
    .unwrap();
    fs::write(
        &loader,
        "\
#!/bin/bash
source \"${BASH_SOURCE[ 0 ]}__dep.bash\"
",
    )
    .unwrap();
    fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
    let unused = reportable_unused_names(&model);
    assert!(unused.is_empty(), "unused: {:?}", unused);
}

#[test]
fn bash_source_nonzero_suffix_does_not_keep_top_level_assignment_live_transitively() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.bash");
    let loader = temp.path().join("loader.bash");
    let helper = temp.path().join("loader.bash__dep.bash");
    fs::write(
        &main,
        "\
#!/bin/bash
flag=1
source ./loader.bash
",
    )
    .unwrap();
    fs::write(
        &loader,
        "\
#!/bin/bash
source \"${BASH_SOURCE[1]}__dep.bash\"
",
    )
    .unwrap();
    fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        !model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
    let unused = reportable_unused_names(&model);
    assert_eq!(unused, vec!["flag"]);
}

#[test]
fn bash_source_dirname_reads_keep_top_level_assignment_live_transitively() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.bash");
    let loader = temp.path().join("loader.bash");
    let helper = temp.path().join("helper.bash");
    fs::write(
        &main,
        "\
#!/bin/bash
flag=1
source ./loader.bash
",
    )
    .unwrap();
    fs::write(
        &loader,
        "\
#!/bin/bash
source \"$(dirname \"${BASH_SOURCE[0]}\")/helper.bash\"
",
    )
    .unwrap();
    fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
    let unused = reportable_unused_names(&model);
    assert!(unused.is_empty(), "unused: {:?}", unused);
}

#[test]
fn executed_helper_reads_keep_loop_variable_live() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
for queryip in 127.0.0.1; do
  helper.sh
done
",
    )
    .unwrap();
    fs::write(&helper, "printf '%s\\n' \"$queryip\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model
            .synthetic_reads
            .iter()
            .any(|read| read.name == "queryip"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
    let unused = reportable_unused_names(&model);
    assert!(
        !unused.contains(&Name::from("queryip")),
        "unused: {:?}",
        unused
    );
}

#[test]
fn executed_helper_without_read_does_not_keep_unrelated_assignment_live() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
unused=1
helper.sh
",
    )
    .unwrap();
    fs::write(&helper, "printf '%s\\n' ok\n").unwrap();

    let model = model_at_path(&main);

    let unused = reportable_unused_names(&model);
    assert!(
        unused.contains(&Name::from("unused")),
        "unused: {:?}",
        unused
    );
}

#[test]
fn loader_function_source_reads_keep_top_level_assignment_live() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
load() { . \"$ROOT/$1\"; }
flag=1
load helper.sh
",
    )
    .unwrap();
    fs::write(&helper, "echo \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
    let unused = reportable_unused_names(&model);
    assert!(unused.is_empty(), "unused: {:?}", unused);
}

#[test]
fn wrapped_loader_function_source_reads_keep_top_level_assignment_live() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
load() { . \"$ROOT/$1\"; }
flag=1
noglob load helper.sh
",
    )
    .unwrap();
    fs::write(&helper, "echo \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
}

#[test]
fn source_path_resolver_keeps_helper_reads_generic() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("resolved/helper.sh");
    fs::create_dir_all(helper.parent().unwrap()).unwrap();
    fs::write(
        &main,
        "\
#!/bin/sh
flag=1
./helper.sh
",
    )
    .unwrap();
    fs::write(&helper, "echo \"$flag\"\n").unwrap();

    let without_resolver = model_at_path(&main);
    let unused_without_resolver = reportable_unused_names(&without_resolver);
    assert!(
        unused_without_resolver.contains(&Name::from("flag")),
        "unused without resolver: {:?}",
        unused_without_resolver
    );

    let main_path = main.clone();
    let helper_path = helper.clone();
    let resolver = move |source_path: &Path, candidate: &str| {
        if source_path == main_path.as_path() && candidate == "./helper.sh" {
            vec![helper_path.clone()]
        } else {
            Vec::new()
        }
    };

    let with_resolver = model_at_path_with_resolver(&main, Some(&resolver));
    assert!(
        with_resolver
            .synthetic_reads
            .iter()
            .any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        with_resolver.synthetic_reads
    );
    let unused_with_resolver = reportable_unused_names(&with_resolver);
    assert!(
        !unused_with_resolver.contains(&Name::from("flag")),
        "unused with resolver: {:?}",
        unused_with_resolver
    );
}

#[test]
fn missing_literal_source_is_marked_unresolved() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    fs::write(&main, "#!/bin/sh\n. ./missing.sh\n").unwrap();

    let model = model_at_path(&main);

    assert_eq!(model.source_refs().len(), 1);
    assert_eq!(
        model.source_refs()[0].resolution,
        SourceRefResolution::Unresolved
    );
}

#[test]
fn resolved_literal_source_is_marked_resolved() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(&main, "#!/bin/sh\n. ./helper.sh\n").unwrap();
    fs::write(&helper, "echo helper\n").unwrap();

    let model = model_at_path(&main);

    assert_eq!(model.source_refs().len(), 1);
    assert_eq!(
        model.source_refs()[0].resolution,
        SourceRefResolution::Resolved
    );
}

#[test]
fn source_path_resolver_can_use_single_variable_static_tails() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("tests/main.sh");
    let helper = temp.path().join("scripts/rvm");
    fs::create_dir_all(main.parent().unwrap()).unwrap();
    fs::create_dir_all(helper.parent().unwrap()).unwrap();
    fs::write(
        &main,
        "\
#!/bin/sh
flag=1
source \"$rvm_path/scripts/rvm\"
",
    )
    .unwrap();
    fs::write(&helper, "echo \"$flag\"\n").unwrap();

    let without_resolver = model_at_path(&main);
    let unused_without_resolver = reportable_unused_names(&without_resolver);
    assert!(
        unused_without_resolver.contains(&Name::from("flag")),
        "unused without resolver: {:?}",
        unused_without_resolver
    );

    let main_path = main.clone();
    let helper_path = helper.clone();
    let resolver = move |source_path: &Path, candidate: &str| {
        if source_path == main_path.as_path() && candidate == "scripts/rvm" {
            vec![helper_path.clone()]
        } else {
            Vec::new()
        }
    };

    let with_resolver = model_at_path_with_resolver(&main, Some(&resolver));
    assert!(
        with_resolver
            .synthetic_reads
            .iter()
            .any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        with_resolver.synthetic_reads
    );
    let unused_with_resolver = reportable_unused_names(&with_resolver);
    assert!(
        !unused_with_resolver.contains(&Name::from("flag")),
        "unused with resolver: {:?}",
        unused_with_resolver
    );
}

#[test]
fn sourced_helper_exports_definite_imported_binding() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
. ./helper.sh
printf '%s\\n' \"$flag\"
",
    )
    .unwrap();
    fs::write(&helper, "flag=1\n").unwrap();

    let model = model_at_path(&main);

    let imported = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "flag" && binding.kind == BindingKind::Imported)
        .unwrap();
    assert!(
        !imported
            .attributes
            .contains(BindingAttributes::IMPORTED_POSSIBLE)
    );
    assert!(model.analysis().uninitialized_references().is_empty());
}

#[test]
fn sourced_helper_exports_possible_imported_binding() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
. ./helper.sh
printf '%s\\n' \"$flag\"
",
    )
    .unwrap();
    fs::write(
        &helper,
        "\
if cond; then
  flag=1
fi
",
    )
    .unwrap();

    let model = model_at_path(&main);
    let imported_is_possible = model
        .bindings()
        .iter()
        .find(|binding| binding.name == "flag" && binding.kind == BindingKind::Imported)
        .map(|binding| {
            binding
                .attributes
                .contains(BindingAttributes::IMPORTED_POSSIBLE)
        })
        .unwrap();
    let details = uninitialized_details(&model);
    assert!(imported_is_possible, "uninitialized: {:?}", details);
    assert_eq!(
        details,
        vec![("flag".to_owned(), UninitializedCertainty::Possible)]
    );
}

#[test]
fn local_shadowing_can_clear_imported_initialization_before_nested_command_substitutions() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/bash
. ./helper.sh
f() {
  local flag
  printf '%s\\n' \"$(
    printf '%s\\n' \"$flag\"
  )\"
}
f
",
    )
    .unwrap();
    fs::write(&helper, "flag=1\n").unwrap();

    let model = model_at_path(&main);
    let uninitialized = uninitialized_names(&model);

    assert_names_present(&["flag"], &uninitialized);
}

#[test]
fn scope_entry_loops_preserve_possible_uninitialized_names() {
    let source = "\
while command; do
  flag=1
done
printf '%s\\n' \"$flag\"
";
    let model = model(source);

    assert_eq!(
        uninitialized_details(&model),
        vec![("flag".to_owned(), UninitializedCertainty::Possible)]
    );
}

#[test]
fn sourced_helper_function_reads_do_not_keep_assignments_live_until_called() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
flag=1
. ./helper.sh
",
    )
    .unwrap();
    fs::write(
        &helper,
        "\
use_flag() {
  printf '%s\\n' \"$flag\"
}
",
    )
    .unwrap();

    let model = model_at_path(&main);
    let unused = reportable_unused_names(&model);
    assert!(unused.contains(&Name::from("flag")), "unused: {:?}", unused);
}

#[test]
fn quoted_heredoc_body_does_not_report_uninitialized_reads() {
    let source = "\
build=\"$(command cat <<\\END
printf '%s\\n' \"$workdir\"
END
)\"
";
    let model = model(source);
    assert!(model.analysis().uninitialized_references().is_empty());
}

#[test]
fn escaped_dollar_heredoc_body_does_not_report_uninitialized_reads() {
    let source = "\
#!/bin/sh
cat <<EOF
\\${devtype} \\${devnum}
EOF
";
    let model = model(source);
    assert!(model.analysis().uninitialized_references().is_empty());
}

#[test]
fn escaped_dollar_word_does_not_report_uninitialized_reads() {
    let source = "\
#!/bin/sh
printf '%s\\n' \"\\$workdir\"
";
    let model = model(source);
    assert!(model.analysis().uninitialized_references().is_empty());
}

#[test]
fn escaped_parameter_expansion_keeps_nested_default_reads() {
    let source = "\
#!/bin/sh
printf '%s\\n' \\${workdir:-$fallback} \\${other:-${inner}}
";
    let model = model(source);
    let unresolved = unresolved_names(&model);

    assert_names_absent(&["workdir", "other"], &unresolved);
    assert_names_present(&["fallback", "inner"], &unresolved);
}

#[test]
fn unquoted_heredoc_body_reports_live_uninitialized_reads() {
    let source = "\
archname=archive
cat <<EOF > \"$archname\"
#!/bin/sh
ORIG_UMASK=`umask`
if test \"$KEEP_UMASK\" = n; then
    umask 077
fi

CRCsum=\"$CRCsum\"
archdirname=\"$archdirname\"
EOF
";
    let model = model(source);
    let details = uninitialized_details(&model);

    assert!(details.iter().any(
        |(name, certainty)| name == "CRCsum" && *certainty == UninitializedCertainty::Definite
    ));
    assert!(
        details.iter().any(|(name, certainty)| name == "archdirname"
            && *certainty == UninitializedCertainty::Definite)
    );
}

#[test]
fn quoted_heredoc_source_text_does_not_keep_assignments_live() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.bash");
    fs::write(
        &main,
        "\
#!/bin/bash
outdir=/tmp
build=\"$(command cat <<\\END
. \\\"$outdir\\\"/build.info
END
)\"
",
    )
    .unwrap();

    let model = model_at_path(&main);
    let unused = reportable_unused_names(&model);
    assert!(
        unused.contains(&Name::from("outdir")),
        "unused: {:?}",
        unused
    );
}

#[test]
fn quoted_heredoc_body_stays_inert_with_source_closure_enabled() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.bash");
    fs::write(
        &main,
        "\
#!/bin/sh
build=\"$(command cat <<\\END
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"
",
    )
    .unwrap();

    let model = model_at_path(&main);
    assert!(
        model.analysis().uninitialized_references().is_empty(),
        "uninitialized: {:?}",
        model.analysis().uninitialized_references()
    );
}

#[test]
fn posix_quoted_heredoc_body_stays_inert_with_source_closure_enabled() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
build=\"$(command cat <<\\END
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"
",
    )
    .unwrap();

    let model = model_at_path_with_parse_dialect(&main, ShellDialect::Posix);
    assert!(
        model.analysis().uninitialized_references().is_empty(),
        "uninitialized: {:?}",
        model.analysis().uninitialized_references()
    );
}

#[test]
fn posix_second_quoted_heredoc_body_stays_inert_with_source_closure_enabled() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
usage=\"$(command cat <<\\END
Usage
END
)\"

build=\"$(command cat <<\\END
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"
",
    )
    .unwrap();

    let model = model_at_path_with_parse_dialect(&main, ShellDialect::Posix);
    assert!(
        model.analysis().uninitialized_references().is_empty(),
        "uninitialized: {:?}",
        model.analysis().uninitialized_references()
    );
}

#[test]
fn quoted_heredoc_build_template_executed_later_stays_inert() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("build.info");
    fs::write(
        &main,
        "\
#!/bin/sh
usage=\"$(command cat <<\\END
Usage
END
)\"

build=\"$(command cat <<\\END
outdir=\"$(command pwd)\"
workdir=\"${TMPDIR:-/tmp}/gitstatus-build.tmp.$$\"\n\
for formula in libiconv cmake git wget; do
  if command brew ls --version \"$formula\" >/dev/null 2>&1; then
    command brew upgrade \"$formula\"
  else
    command brew install \"$formula\"
  fi
done
archflag=\"-march\"
nopltflag=\"-fno-plt\"
cflags=\"$archflag=$cpu $nopltflag\"
. \"$outdir\"/build.info
END
)\"

eval \"$build\"
",
    )
    .unwrap();
    fs::write(&helper, "libgit2_version=1.0\n").unwrap();

    let model = model_at_path(&main);
    let references = model.analysis().uninitialized_references().to_vec();
    let names = references
        .iter()
        .map(|reference| model.reference(reference.reference).name.clone())
        .collect::<Vec<_>>();
    assert!(
        !names.iter().any(|name| {
            matches!(
                name.as_str(),
                "formula" | "archflag" | "nopltflag" | "outdir"
            )
        }),
        "uninitialized names: {names:?}"
    );
}

#[test]
fn eval_scan_keeps_apostrophe_inside_double_quotes_from_hiding_reads() {
    let source = "foo=1\neval \"echo \\\"it's \\$foo\\\"\"\n";
    let model = model(source);

    let unused = reportable_unused_names(&model);
    assert!(
        !unused.contains(&Name::from("foo")),
        "unused bindings: {:?}",
        unused
    );
}

#[test]
fn eval_scan_keeps_quoted_hash_from_starting_comment() {
    let source = "foo=1\neval \"echo \\\"# \\$foo\\\"\"\n";
    let model = model(source);

    let unused = reportable_unused_names(&model);
    assert!(
        !unused.contains(&Name::from("foo")),
        "unused bindings: {:?}",
        unused
    );
}

#[test]
fn eval_scan_treats_unquoted_hash_after_space_as_comment() {
    let source = "foo=1\neval \"echo # \\$foo\"\n";
    let model = model(source);

    let unused = reportable_unused_names(&model);
    assert!(
        unused.contains(&Name::from("foo")),
        "unused bindings: {:?}",
        unused
    );
}

#[test]
fn eval_scan_keeps_source_single_quoted_fragments_inert_for_c001() {
    let source = "foo=1\neval 'echo \"$foo\"'\n";
    let model = model(source);

    let unused = reportable_unused_names(&model);
    assert!(
        unused.contains(&Name::from("foo")),
        "unused bindings: {:?}",
        unused
    );
}

#[test]
fn eval_scan_treats_pid_prefix_as_special_parameter() {
    let source = "foo=1\neval \"echo $$foo\"\n";
    let model = model(source);

    let unused = reportable_unused_names(&model);
    assert!(
        unused.contains(&Name::from("foo")),
        "unused bindings: {:?}",
        unused
    );
}

#[test]
fn escaped_dollar_heredoc_body_stays_inert_with_source_closure_enabled() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
cat <<EOF > ./postinst
if [ \"\\$1\" = \"configure\" ]; then
  for ver in 1 current; do
    for x in rewriteSystem rewriteURI; do
      xmlcatalog --noout --add \\$x http://example.test/xsl/\\$ver
    done
  done
fi
EOF
",
    )
    .unwrap();

    let model = model_at_path(&main);
    assert!(
        model.analysis().uninitialized_references().is_empty(),
        "uninitialized: {:?}",
        model.analysis().uninitialized_references()
    );
}

#[test]
fn quoted_heredoc_case_arm_and_nested_same_name_heredoc_stay_inert() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
build=\"$(command cat <<\\END
case \"$gitstatus_kernel\" in
  linux)
    for formula in libiconv cmake git wget; do
      if command brew ls --version \"$formula\" >/dev/null; then
        command brew upgrade \"$formula\"
      else
        command brew install \"$formula\"
      fi
    done
  ;;
esac
command cat >&2 <<-END
\tSUCCESS
\tEND
END
)\"
",
    )
    .unwrap();

    let model = model_at_path(&main);
    assert!(
        model.analysis().uninitialized_references().is_empty(),
        "uninitialized: {:?}",
        model.analysis().uninitialized_references()
    );
}

#[test]
fn tab_stripped_escaped_dollar_heredoc_body_stays_inert() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
cat <<- EOF > ./postinst
\tif [ \"\\$1\" = \"configure\" ]; then
\t\tfor ver in 1 current; do
\t\t\tfor x in rewriteSystem rewriteURI; do
\t\t\t\txmlcatalog --noout --add \\$x http://example.test/xsl/\\$ver
\t\t\tdone
\t\tdone
\tfi
\tEOF
",
    )
    .unwrap();

    let model = model_at_path(&main);
    assert!(
        model.analysis().uninitialized_references().is_empty(),
        "uninitialized: {:?}",
        model.analysis().uninitialized_references()
    );
}

#[test]
fn posix_tab_stripped_escaped_dollar_heredoc_body_stays_inert() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
cat <<- EOF > ./postinst
\tif [ \"$TERMUX_PACKAGE_FORMAT\" = \"pacman\" ] || [ \"\\$1\" = \"configure\" ]; then
\t\tfor ver in $TERMUX_PKG_VERSION current; do
\t\t\tfor x in rewriteSystem rewriteURI; do
\t\t\t\txmlcatalog --noout --add \\$x http://docbook.sourceforge.net/release/xsl-ns/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-$TERMUX_PKG_VERSION\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\t\t\tdone
\t\tdone
\tfi
\tEOF
",
    )
    .unwrap();

    let model = model_at_path_with_parse_dialect(&main, ShellDialect::Posix);
    let references = model.analysis().uninitialized_references().to_vec();
    let names = references
        .iter()
        .map(|reference| model.reference(reference.reference).name.clone())
        .collect::<Vec<_>>();
    assert!(
        !names
            .iter()
            .any(|name| matches!(name.as_str(), "x" | "ver")),
        "uninitialized names: {names:?}"
    );
}

#[test]
fn posix_docbook_wrapper_does_not_treat_escaped_placeholders_as_reads() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
termux_step_create_debscripts() {
\tcat <<- EOF > ./postinst
\t#!$TERMUX_PREFIX/bin/sh
\tif [ \"$TERMUX_PACKAGE_FORMAT\" = \"pacman\" ] || [ \"\\$1\" = \"configure\" ]; then
\t\tfor ver in $TERMUX_PKG_VERSION current; do
\t\t\tfor x in rewriteSystem rewriteURI; do
\t\t\t\txmlcatalog --noout --add \\$x http://cdn.docbook.org/release/xsl/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-$TERMUX_PKG_VERSION\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\
\t\t\t\txmlcatalog --noout --add \\$x http://docbook.sourceforge.net/release/xsl-ns/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-$TERMUX_PKG_VERSION\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\
\t\t\t\txmlcatalog --noout --add \\$x http://docbook.sourceforge.net/release/xsl/\\$ver \\
\t\t\t\t\t\"$TERMUX_PREFIX/share/xml/docbook/xsl-stylesheets-${TERMUX_PKG_VERSION}-nons\" \\
\t\t\t\t\t\"$TERMUX_PREFIX/etc/xml/catalog\"
\t\t\tdone
\t\tdone
\tfi
\tEOF
}
",
    )
    .unwrap();

    let model = model_at_path_with_parse_dialect(&main, ShellDialect::Posix);
    let references = model.analysis().uninitialized_references().to_vec();
    let names = references
        .iter()
        .map(|reference| model.reference(reference.reference).name.clone())
        .collect::<Vec<_>>();
    assert!(
        !names
            .iter()
            .any(|name| matches!(name.as_str(), "x" | "ver")),
        "uninitialized names: {names:?}"
    );
}

#[test]
fn sourced_helper_function_reads_keep_assignments_live_when_called() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
flag=1
. ./helper.sh
use_flag
",
    )
    .unwrap();
    fs::write(
        &helper,
        "\
use_flag() {
  printf '%s\\n' \"$flag\"
}
",
    )
    .unwrap();

    let model = model_at_path(&main);
    let unused = reportable_unused_names(&model);
    assert!(
        !unused.contains(&Name::from("flag")),
        "unused: {:?}",
        unused
    );
}

#[test]
fn sourced_helper_function_exports_definite_imported_binding_when_called() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
. ./helper.sh
set_flag
printf '%s\\n' \"$flag\"
",
    )
    .unwrap();
    fs::write(
        &helper,
        "\
set_flag() {
  flag=1
}
",
    )
    .unwrap();

    let model = model_at_path(&main);
    assert!(model.analysis().uninitialized_references().is_empty());
}

#[test]
fn sourced_helper_function_exports_possible_imported_binding_when_called() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
. ./helper.sh
set_flag
printf '%s\\n' \"$flag\"
",
    )
    .unwrap();
    fs::write(
        &helper,
        "\
set_flag() {
  if cond; then
    flag=1
  fi
}
",
    )
    .unwrap();

    let model = model_at_path(&main);
    assert_eq!(
        uninitialized_details(&model),
        vec![("flag".to_owned(), UninitializedCertainty::Possible)]
    );
}

#[test]
fn layered_source_closure_imports_function_contracts_transitively() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let loader = temp.path().join("loader.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
. ./loader.sh
set_flag
printf '%s\\n' \"$flag\"
",
    )
    .unwrap();
    fs::write(&loader, ". ./helper.sh\n").unwrap();
    fs::write(
        &helper,
        "\
set_flag() {
  flag=1
}
",
    )
    .unwrap();

    let model = model_at_path(&main);
    assert!(model.analysis().uninitialized_references().is_empty());
}

#[test]
fn late_parent_scope_loader_calls_are_visible_from_nested_functions() {
    let source = "\
outer() {
  inner() { load_helper ./helper.sh; }
  load_helper() { . \"$1\"; }
  inner
}
";
    let model = model(source);
    let name = Name::from("load_helper");
    let call = &model.call_sites_for(&name)[0];
    assert!(
        model
            .analysis()
            .visible_function_binding_at_call(&name, call.name_span)
            .is_some()
    );
}

#[test]
fn executed_helper_does_not_import_bindings_back_to_the_caller() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
./helper.sh
printf '%s\\n' \"$flag\"
",
    )
    .unwrap();
    fs::write(&helper, "flag=1\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model
            .bindings()
            .iter()
            .all(|binding| !(binding.name == "flag" && binding.kind == BindingKind::Imported))
    );
    assert_eq!(
        uninitialized_details(&model),
        vec![("flag".to_owned(), UninitializedCertainty::Definite)]
    );
}

#[test]
fn imported_bindings_do_not_resolve_reads_before_the_import_site() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let helper = temp.path().join("helper.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
printf '%s\\n' \"$flag\"
. ./helper.sh
",
    )
    .unwrap();
    fs::write(&helper, "flag=1\n").unwrap();

    let model = model_at_path(&main);
    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "flag")
        .unwrap();

    assert!(model.resolved_binding(reference.id).is_none());
    assert_eq!(
        uninitialized_details(&model),
        vec![("flag".to_owned(), UninitializedCertainty::Definite)]
    );
}

#[test]
fn file_entry_contracts_seed_multiple_first_command_reads_as_imported_bindings() {
    let source = "printf '%s\\n' \"$pkgname\" \"$pkgver\" \"$wrksrc\"\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build_with_options(
        &output.file,
        source,
        &indexer,
        SemanticBuildOptions {
            file_entry_contract: Some(FileContract {
                required_reads: Vec::new(),
                provided_bindings: vec![
                    ProvidedBinding::new(
                        Name::from("pkgname"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    ),
                    ProvidedBinding::new(
                        Name::from("pkgver"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    ),
                    ProvidedBinding::new(
                        Name::from("wrksrc"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    ),
                ],
                provided_functions: Vec::new(),
                externally_consumed_bindings: false,
            }),
            ..SemanticBuildOptions::default()
        },
    );

    for name in ["pkgname", "pkgver", "wrksrc"] {
        let reference = model
            .references()
            .iter()
            .find(|reference| reference.name == name)
            .unwrap();
        let binding = model.resolved_binding(reference.id).unwrap();
        assert_eq!(binding.kind, BindingKind::Imported);
        assert_eq!(binding.name, name);
        assert!(
            binding
                .attributes
                .contains(BindingAttributes::IMPORTED_FILE_ENTRY)
        );
    }
    assert_eq!(
        uninitialized_details(&model),
        vec![
            ("pkgname".to_owned(), UninitializedCertainty::Definite),
            ("pkgver".to_owned(), UninitializedCertainty::Definite),
            ("wrksrc".to_owned(), UninitializedCertainty::Definite),
        ]
    );
}

#[test]
fn file_entry_contracts_seed_deferred_function_body_reads_as_imported_bindings() {
    let source = "\
build() {
  printf '%s\\n' \"$pkgname\" \"$pkgver\" \"$wrksrc\"
}
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build_with_options(
        &output.file,
        source,
        &indexer,
        SemanticBuildOptions {
            file_entry_contract: Some(FileContract {
                required_reads: Vec::new(),
                provided_bindings: vec![
                    ProvidedBinding::new(
                        Name::from("pkgname"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    ),
                    ProvidedBinding::new(
                        Name::from("pkgver"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    ),
                    ProvidedBinding::new(
                        Name::from("wrksrc"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    ),
                ],
                provided_functions: Vec::new(),
                externally_consumed_bindings: false,
            }),
            ..SemanticBuildOptions::default()
        },
    );

    for name in ["pkgname", "pkgver", "wrksrc"] {
        let reference = model
            .references()
            .iter()
            .find(|reference| reference.name == name && reference.kind == ReferenceKind::Expansion)
            .unwrap();
        let binding = model.resolved_binding(reference.id).unwrap();
        assert_eq!(binding.kind, BindingKind::Imported);
        assert_eq!(binding.name, name);
        assert!(
            binding
                .attributes
                .contains(BindingAttributes::IMPORTED_FILE_ENTRY)
        );
    }
    assert_eq!(
        uninitialized_details(&model),
        vec![
            ("pkgname".to_owned(), UninitializedCertainty::Definite),
            ("pkgver".to_owned(), UninitializedCertainty::Definite),
            ("wrksrc".to_owned(), UninitializedCertainty::Definite),
        ]
    );
}

#[test]
fn file_entry_contracts_seed_nested_function_regions_as_imported_bindings() {
    let source = "\
hook() {
  for f in ${pycompile_dirs}; do
    if [ \"${pkgname}\" = \"base-files\" ]; then
      echo \"python${pycompile_version}\"
    else
      printf '%s\\n' \"${pkgver}: ${f}\"
    fi
  done
}
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build_with_options(
        &output.file,
        source,
        &indexer,
        SemanticBuildOptions {
            file_entry_contract: Some(FileContract {
                required_reads: Vec::new(),
                provided_bindings: vec![
                    ProvidedBinding::new(
                        Name::from("pkgname"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    ),
                    ProvidedBinding::new(
                        Name::from("pkgver"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    ),
                    ProvidedBinding::new(
                        Name::from("pycompile_dirs"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    ),
                    ProvidedBinding::new(
                        Name::from("pycompile_version"),
                        ProvidedBindingKind::Variable,
                        ContractCertainty::Definite,
                    ),
                ],
                provided_functions: Vec::new(),
                externally_consumed_bindings: false,
            }),
            ..SemanticBuildOptions::default()
        },
    );

    for name in ["pkgname", "pkgver", "pycompile_dirs", "pycompile_version"] {
        let reference = model
            .references()
            .iter()
            .find(|reference| reference.name == name)
            .unwrap();
        let binding = model.resolved_binding(reference.id).unwrap();
        assert_eq!(binding.kind, BindingKind::Imported);
        assert_eq!(binding.name, name);
        assert!(
            binding
                .attributes
                .contains(BindingAttributes::IMPORTED_FILE_ENTRY)
        );
    }
    assert_eq!(
        uninitialized_details(&model),
        vec![
            (
                "pycompile_dirs".to_owned(),
                UninitializedCertainty::Definite
            ),
            ("pkgname".to_owned(), UninitializedCertainty::Definite),
            (
                "pycompile_version".to_owned(),
                UninitializedCertainty::Definite
            ),
            ("pkgver".to_owned(), UninitializedCertainty::Definite),
        ]
    );
}

#[test]
fn initialized_file_entry_bindings_suppress_uninitialized_reads() {
    let source = "printf '%s\\n' \"$theme_color\"\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build_with_options(
        &output.file,
        source,
        &indexer,
        SemanticBuildOptions {
            file_entry_contract: Some(FileContract {
                required_reads: Vec::new(),
                provided_bindings: vec![ProvidedBinding::new_file_entry_initialized(
                    Name::from("theme_color"),
                    ProvidedBindingKind::Variable,
                    ContractCertainty::Definite,
                )],
                provided_functions: Vec::new(),
                externally_consumed_bindings: false,
            }),
            ..SemanticBuildOptions::default()
        },
    );

    let reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "theme_color")
        .unwrap();
    let binding = model.resolved_binding(reference.id).unwrap();
    assert!(
        binding
            .attributes
            .contains(BindingAttributes::IMPORTED_FILE_ENTRY_INITIALIZED)
    );
    assert!(uninitialized_names(&model).is_empty());
}

#[test]
fn file_entry_contracts_can_mark_assignments_as_caller_consumed() {
    let source = "published=1\n";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build_with_options(
        &output.file,
        source,
        &indexer,
        SemanticBuildOptions {
            file_entry_contract: Some(FileContract {
                required_reads: Vec::new(),
                provided_bindings: Vec::new(),
                provided_functions: Vec::new(),
                externally_consumed_bindings: true,
            }),
            ..SemanticBuildOptions::default()
        },
    );

    let binding = binding_for_name(&model, "published");
    assert!(
        binding
            .attributes
            .contains(BindingAttributes::EXTERNALLY_CONSUMED)
    );
    assert!(model.analysis().unused_assignments().is_empty());
}

#[test]
fn cyclic_source_closure_does_not_invent_bindings() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.sh");
    let a = temp.path().join("a.sh");
    let b = temp.path().join("b.sh");
    fs::write(
        &main,
        "\
#!/bin/sh
. ./a.sh
printf '%s\\n' \"$flag\"
",
    )
    .unwrap();
    fs::write(&a, ". ./b.sh\n").unwrap();
    fs::write(&b, ". ./a.sh\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model
            .bindings()
            .iter()
            .all(|binding| !(binding.name == "flag" && binding.kind == BindingKind::Imported))
    );
    assert_eq!(
        uninitialized_details(&model),
        vec![("flag".to_owned(), UninitializedCertainty::Definite)]
    );
}

#[test]
fn unsupported_bash_source_alias_fallback_does_not_keep_assignment_live() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.bash");
    let loader = temp.path().join("loader.bash");
    let helper = temp.path().join("helper.bash");
    fs::write(
        &main,
        "\
#!/bin/bash
flag=1
source ./loader.bash
",
    )
    .unwrap();
    fs::write(
        &loader,
        "\
#!/bin/bash
SELF=\"${BASH_SOURCE}\"
source \"$(dirname \"${SELF:-$0}\")/helper.bash\"
",
    )
    .unwrap();
    fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        !model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
    let unused = reportable_unused_names(&model);
    assert!(unused.contains(&Name::from("flag")), "unused: {:?}", unused);
}

#[test]
fn escaped_bash_source_template_does_not_import_helper() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.bash");
    let helper = temp.path().join("helper.bash");
    fs::write(
        &main,
        "\
#!/bin/bash
flag=1
source \"\\$(dirname \\\"${BASH_SOURCE[0]}\\\")/helper.bash\"
",
    )
    .unwrap();
    fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        !model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
    let unused = reportable_unused_names(&model);
    assert!(unused.contains(&Name::from("flag")), "unused: {:?}", unused);
}

#[test]
fn shellcheck_source_directive_overrides_bash_source_template() {
    let temp = tempdir().unwrap();
    let main = temp.path().join("main.bash");
    let loader = temp.path().join("loader.bash");
    let helper = temp.path().join("alt-helper.bash");
    fs::write(
        &main,
        "\
#!/bin/bash
flag=1
source ./loader.bash
",
    )
    .unwrap();
    fs::write(
        &loader,
        "\
#!/bin/bash
# shellcheck source=alt-helper.bash
source \"$(dirname \"${BASH_SOURCE[0]}\")/missing-helper.bash\"
",
    )
    .unwrap();
    fs::write(&helper, "#!/bin/bash\necho \"$flag\"\n").unwrap();

    let model = model_at_path(&main);

    assert!(
        model.synthetic_reads.iter().any(|read| read.name == "flag"),
        "synthetic reads: {:?}",
        model.synthetic_reads
    );
    assert_eq!(
        model.source_refs()[0].resolution,
        SourceRefResolution::Resolved
    );
    let unused = reportable_unused_names(&model);
    assert!(unused.is_empty(), "unused: {:?}", unused);
}

#[test]
fn precise_unused_assignments_match_dataflow_for_source_closure_cases() {
    let temp = tempdir().unwrap();

    let sourced_main = temp.path().join("sourced-main.sh");
    let sourced_helper = temp.path().join("sourced-helper.sh");
    fs::write(
        &sourced_main,
        "\
#!/bin/sh
flag=1
. ./sourced-helper.sh
",
    )
    .unwrap();
    fs::write(&sourced_helper, "echo \"$flag\"\n").unwrap();

    let executed_main = temp.path().join("executed-main.sh");
    let executed_helper = temp.path().join("executed-helper.sh");
    fs::write(
        &executed_main,
        "\
#!/bin/sh
unused=1
executed-helper.sh
",
    )
    .unwrap();
    fs::write(&executed_helper, "printf '%s\\n' ok\n").unwrap();

    let sourced_model = model_at_path(&sourced_main);
    assert_unused_assignment_parity(&sourced_model);

    let executed_model = model_at_path(&executed_main);
    assert_unused_assignment_parity(&executed_model);
}

#[test]
fn non_arithmetic_subscript_reads_are_recorded_in_conditionals_and_declarations() {
    let source = "\
#!/bin/bash
[[ -v assoc[\"$key\"] ]]
declare -A map=([\"$other\"]=1)
";
    let model = model(source);
    let unresolved = unresolved_names(&model);

    assert_names_present(&["key", "other"], &unresolved);

    let conditional_reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "key")
        .expect("expected conditional subscript reference");
    assert_eq!(
        conditional_reference.kind,
        ReferenceKind::ConditionalOperand
    );

    let declaration_reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "other")
        .expect("expected declaration subscript reference");
    assert_eq!(declaration_reference.kind, ReferenceKind::Expansion);
}

#[test]
fn associative_subscript_literals_do_not_register_variable_reads() {
    let source = "\
#!/bin/bash
declare -A map
map[swift-cmark]=1
printf '%s\\n' \"${map[swift-cmark]}\" \"${map[$dynamic_key]}\"
";
    let model = model(source);
    let unresolved = unresolved_names(&model);

    assert_names_absent(&["swift", "cmark"], &unresolved);
    assert_names_present(&["dynamic_key"], &unresolved);
    assert!(
        model
            .bindings()
            .iter()
            .rev()
            .find(|binding| binding.name == "map")
            .is_some_and(|binding| binding.attributes.contains(BindingAttributes::ASSOC))
    );
}

#[test]
fn associative_arithmetic_subscript_literals_do_not_register_variable_reads() {
    let source = "\
#!/bin/bash
declare -A box
printf '%s\\n' \"$((box[m_width]))\" \"$((box[$dynamic_key]))\"
";
    let model = model(source);
    let unresolved = unresolved_names(&model);

    assert_names_absent(&["m_width"], &unresolved);
    assert_names_present(&["dynamic_key"], &unresolved);
}

#[test]
fn zsh_arithmetic_option_keys_do_not_register_key_reads_or_updates_without_assoc_binding() {
    let source = "\
#!/bin/zsh
f() {
  local quiet=0
  ( (( !OPTS[opt_-q,--quiet] )) )
  (( quiet ))
}
";
    let model = model_with_dialect(source, ShellDialect::Zsh);
    let unresolved = unresolved_names(&model);

    assert_names_absent(&["opt_", "q"], &unresolved);
    assert_arithmetic_usage(&model, "quiet", 1, 0);
}

#[test]
fn zsh_regular_arithmetic_indices_still_register_updates() {
    let source = "\
#!/bin/zsh
f() {
  local i=1
  (( arr[--i] ))
}
";
    let model = model_with_dialect(source, ShellDialect::Zsh);

    assert_arithmetic_usage(&model, "i", 1, 1);
}

#[test]
fn zsh_arithmetic_comma_indices_still_register_updates() {
    let source = "\
#!/bin/zsh
f() {
  local i=1 quiet=1
  (( arr[i,--quiet] ))
}
";
    let model = model_with_dialect(source, ShellDialect::Zsh);

    assert_arithmetic_usage(&model, "i", 1, 0);
    assert_arithmetic_usage(&model, "quiet", 1, 1);
}

#[test]
fn zsh_option_shaped_non_opts_indices_still_register_updates() {
    let source = "\
#!/bin/zsh
f() {
  local opt_=1 q=1 quiet=1
  (( arr[opt_-q,--quiet] ))
}
";
    let model = model_with_dialect(source, ShellDialect::Zsh);

    assert_arithmetic_usage(&model, "opt_", 1, 0);
    assert_arithmetic_usage(&model, "q", 1, 0);
    assert_arithmetic_usage(&model, "quiet", 1, 1);
}

#[test]
fn zsh_option_shaped_indexed_opts_indices_still_register_updates() {
    let source = "\
#!/bin/zsh
f() {
  local -a OPTS
  local opt_=1 q=1 quiet=1
  (( OPTS[opt_-q,--quiet] ))
}
";
    let model = model_with_dialect(source, ShellDialect::Zsh);

    assert_arithmetic_usage(&model, "opt_", 1, 0);
    assert_arithmetic_usage(&model, "q", 1, 0);
    assert_arithmetic_usage(&model, "quiet", 1, 1);
}

#[test]
fn zsh_option_shaped_caller_indexed_opts_indices_still_register_updates() {
    let source = "\
#!/bin/zsh
caller() {
  local -a OPTS
  callee
}
callee() {
  local opt_=1 q=1 quiet=1
  (( OPTS[opt_-q,--quiet] ))
}
caller
";
    let model = model_with_dialect(source, ShellDialect::Zsh);

    assert_arithmetic_usage(&model, "opt_", 1, 0);
    assert_arithmetic_usage(&model, "q", 1, 0);
    assert_arithmetic_usage(&model, "quiet", 1, 1);
}

#[test]
fn arithmetic_indexed_writes_preserve_associative_attributes() {
    let source = "\
#!/bin/bash
declare -A box
(( box[key] = 1 ))
printf '%s\\n' \"$((box[m_width]))\" \"$((box[$dynamic_key]))\"
";
    let model = model(source);
    let unresolved = unresolved_names(&model);

    assert_names_absent(&["key", "m_width"], &unresolved);
    assert_names_present(&["dynamic_key"], &unresolved);

    let arithmetic_binding = model
        .bindings()
        .iter()
        .rev()
        .find(|binding| binding.name == "box" && binding.kind == BindingKind::ArithmeticAssignment)
        .expect("expected arithmetic box binding");
    assert!(
        arithmetic_binding
            .attributes
            .contains(BindingAttributes::ARRAY)
    );
    assert!(
        arithmetic_binding
            .attributes
            .contains(BindingAttributes::ASSOC)
    );
}

#[test]
fn parameter_default_subscript_after_unset_does_not_inherit_associative_attributes() {
    let source = "\
#!/bin/bash
declare -A map
unset map
: \"${map[$key]:=}\"
";
    let model = model(source);

    let binding = model
        .bindings()
        .iter()
        .rev()
        .find(|binding| {
            binding.name == "map" && binding.kind == BindingKind::ParameterDefaultAssignment
        })
        .expect("expected parameter-default map binding");
    assert!(binding.attributes.contains(BindingAttributes::ARRAY));
    assert!(!binding.attributes.contains(BindingAttributes::ASSOC));
}

#[test]
fn parameter_default_subscript_after_function_unset_does_not_inherit_global_assoc() {
    let source = "\
#!/bin/bash
declare -A map
f() {
  unset map
  : \"${map[$key]:=}\"
}
f
";
    let model = model(source);

    let binding = model
        .bindings()
        .iter()
        .rev()
        .find(|binding| {
            binding.name == "map" && binding.kind == BindingKind::ParameterDefaultAssignment
        })
        .expect("expected parameter-default map binding");
    assert!(binding.attributes.contains(BindingAttributes::ARRAY));
    assert!(!binding.attributes.contains(BindingAttributes::ASSOC));
}

#[test]
fn deferred_parameter_default_after_function_unset_does_not_inherit_later_global_assoc() {
    let source = "\
#!/bin/bash
f() {
  unset map
  : \"${map[$key]:=}\"
}
declare -A map
f
";
    let model = model(source);

    let binding = model
        .bindings()
        .iter()
        .rev()
        .find(|binding| {
            binding.name == "map" && binding.kind == BindingKind::ParameterDefaultAssignment
        })
        .expect("expected parameter-default map binding");
    assert!(binding.attributes.contains(BindingAttributes::ARRAY));
    assert!(!binding.attributes.contains(BindingAttributes::ASSOC));
}

#[test]
fn deferred_parameter_default_after_global_unset_does_not_inherit_later_global_assoc() {
    let source = "\
#!/bin/bash
f() {
  : \"${map[$key]:=}\"
}
declare -A map
unset map
f
";
    let model = model(source);

    let binding = model
        .bindings()
        .iter()
        .rev()
        .find(|binding| {
            binding.name == "map" && binding.kind == BindingKind::ParameterDefaultAssignment
        })
        .expect("expected parameter-default map binding");
    assert!(binding.attributes.contains(BindingAttributes::ARRAY));
    assert!(!binding.attributes.contains(BindingAttributes::ASSOC));
}

#[test]
fn escaped_parameter_replacement_patterns_do_not_register_variable_reads() {
    let source = "\
#!/bin/bash
d=lib
origin=/tmp
echo \"${d//\\$ORIGIN/$origin}\"
";
    let model = model(source);
    let unresolved = unresolved_names(&model);

    assert!(
        unresolved.is_empty(),
        "unexpected unresolved refs: {unresolved:?}"
    );
}

#[test]
fn parameter_replacement_pattern_reads_do_not_count_as_uninitialized() {
    let source = "\
#!/bin/bash
dir=all/retroarch.cfg
echo \"${dir//$configdir\\/}\"
echo \"${dir##$trim_prefix}\"
find \"$configdir\"
";
    let model = model(source);
    assert!(model.references().iter().any(|reference| {
        reference.name == "configdir"
            && reference.kind == ReferenceKind::ParameterPattern
            && reference.span.slice(source) == "$configdir"
    }));
    assert!(model.references().iter().any(|reference| {
        reference.name == "trim_prefix"
            && reference.kind == ReferenceKind::ParameterPattern
            && reference.span.slice(source) == "$trim_prefix"
    }));

    let analysis = model.analysis();
    let uninitialized = analysis.uninitialized_references();
    assert!(uninitialized.iter().any(|uninitialized| {
        let reference = model.reference(uninitialized.reference);
        reference.name == "configdir" && reference.span.slice(source) == "$configdir"
    }));
    assert!(!uninitialized.iter().any(|uninitialized| {
        let reference = model.reference(uninitialized.reference);
        reference.name == "configdir" && reference.kind == ReferenceKind::ParameterPattern
    }));
    assert!(!uninitialized.iter().any(|uninitialized| {
        let reference = model.reference(uninitialized.reference);
        reference.name == "trim_prefix" && reference.kind == ReferenceKind::ParameterPattern
    }));
}

#[test]
fn redirect_target_references_are_uninitialized_reads() {
    let source = "\
#!/bin/bash
{ echo value; } >> \"${missing_target}/out\"
echo \"${ordinary_missing}/out\"
";
    let model = model(source);
    let redirect_reference = model
        .references()
        .iter()
        .find(|reference| reference.name == "missing_target")
        .expect("redirect target reference should be recorded");
    block_with_reference(model.analysis().cfg(), redirect_reference.id);
    let uninitialized = uninitialized_names(&model);

    assert_names_present(&["missing_target", "ordinary_missing"], &uninitialized);
}

#[test]
fn references_in_command_span_filters_to_direct_references_in_the_requested_subspan() {
    let source = "\
#!/bin/bash
foo=\"$foo\" cmd \"$foo\" \"$(printf '%s' \"$foo\")\"
";
    let output = Parser::new(source).parse().unwrap();
    let model = model(source);
    let command_id = command_id_starting_with(&model, source, "foo=\"$foo\" cmd").unwrap();
    let command_span = model.command_span(command_id);
    let Command::Simple(command) = &output.file.body[0].command else {
        panic!("expected simple command");
    };
    let assignment_span = command.assignments[0].span;
    let body_word_span = command.args[0].span;

    let assignment_refs = model
        .references_in_command_span(command_span, assignment_span)
        .map(|reference| reference.span.slice(source))
        .collect::<Vec<_>>();
    let body_refs = model
        .references_in_command_span(command_span, body_word_span)
        .map(|reference| reference.span.slice(source))
        .collect::<Vec<_>>();
    let command_refs = model
        .references_in_command_span(command_span, command_span)
        .map(|reference| reference.span.slice(source))
        .collect::<Vec<_>>();

    assert_eq!(assignment_refs, vec!["$foo"]);
    assert_eq!(body_refs, vec!["$foo"]);
    assert_eq!(command_refs, vec!["$foo", "$foo"]);
}

#[test]
fn unreachable_references_are_still_uninitialized_reads() {
    let source = "\
#!/bin/bash
load_value() {
  return 1
  printf '%s\\n' \"$after_return\"
}
load_value
";
    let model = model(source);
    let uninitialized = uninitialized_names(&model);

    assert_names_present(&["after_return"], &uninitialized);
}

#[test]
fn uninitialized_reference_certainty_lookup_matches_analysis_results() {
    let source = "\
#!/bin/bash
echo \"$definite\"
case \"$1\" in
  yes)
    possible=1
    ;;
esac
echo \"$possible\"
";
    let model = model(source);
    let analysis = model.analysis();
    let references = analysis.uninitialized_references().to_vec();

    assert!(
        !references.is_empty(),
        "expected representative uninitialized references"
    );

    for uninitialized in references {
        let reference = model.reference(uninitialized.reference);
        assert_eq!(
            analysis.uninitialized_reference_certainty_at(reference.span),
            Some(uninitialized.certainty),
            "certainty lookup should match analysis results for {}",
            reference.name
        );
    }
}

#[test]
fn recorded_program_and_cfg_capture_non_arithmetic_var_ref_nested_regions() {
    let source = "\
[[ -v assoc[\"$(printf inner)\"] ]]
echo done
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build(&output.file, source, &indexer);

    let file_commands = model
        .recorded_program
        .commands_in(model.recorded_program.file_commands());
    assert_eq!(file_commands.len(), 2);
    let conditional = model.recorded_program.command(file_commands[0]);
    let nested_regions = model
        .recorded_program
        .nested_regions(conditional.nested_regions);
    assert_eq!(nested_regions.len(), 1);
    let nested_commands = model
        .recorded_program
        .commands_in(nested_regions[0].commands);
    assert_eq!(nested_commands.len(), 1);
    let nested = model.recorded_program.command(nested_commands[0]);
    assert_eq!(nested.span.slice(source), "printf inner");

    let cfg = build_control_flow_graph(
        &model.recorded_program,
        &model.command_bindings,
        &model.command_references,
        &model.scopes,
        &model.bindings,
        &model.call_sites,
        model.visible_function_call_bindings(),
    );

    assert!(!cfg.block_ids_for_span(conditional.span).is_empty());
    assert!(!cfg.block_ids_for_span(nested.span).is_empty());
    assert!(
        cfg.blocks()
            .iter()
            .flat_map(|block| block.commands.iter())
            .any(|span| span.slice(source) == "printf inner")
    );
}

#[test]
fn recorded_program_and_cfg_capture_arithmetic_var_ref_nested_regions() {
    let source = "\
[[ -v assoc[$(( $(printf inner) + 1 ))] ]]
echo done
";
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let model = SemanticModel::build(&output.file, source, &indexer);

    let file_commands = model
        .recorded_program
        .commands_in(model.recorded_program.file_commands());
    assert_eq!(file_commands.len(), 2);
    let conditional = model.recorded_program.command(file_commands[0]);
    let nested_regions = model
        .recorded_program
        .nested_regions(conditional.nested_regions);
    assert_eq!(nested_regions.len(), 1);
    let nested_commands = model
        .recorded_program
        .commands_in(nested_regions[0].commands);
    assert_eq!(nested_commands.len(), 1);
    let nested = model.recorded_program.command(nested_commands[0]);
    assert_eq!(nested.span.slice(source), "printf inner");

    let cfg = build_control_flow_graph(
        &model.recorded_program,
        &model.command_bindings,
        &model.command_references,
        &model.scopes,
        &model.bindings,
        &model.call_sites,
        model.visible_function_call_bindings(),
    );

    assert!(!cfg.block_ids_for_span(conditional.span).is_empty());
    assert!(!cfg.block_ids_for_span(nested.span).is_empty());
    assert!(
        cfg.blocks()
            .iter()
            .flat_map(|block| block.commands.iter())
            .any(|span| span.slice(source) == "printf inner")
    );
}

#[test]
fn zsh_option_analysis_exposes_native_defaults() {
    let source = "print $name\n";
    let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
    let options = model
        .zsh_options_at(source.find("print").unwrap())
        .expect("expected zsh options");

    assert_eq!(options.sh_word_split, OptionValue::Off);
    assert_eq!(options.glob, OptionValue::On);
    assert_eq!(options.short_loops, OptionValue::On);
}

#[test]
fn zsh_option_analysis_tracks_setopt_updates_by_offset() {
    let source = "setopt no_glob\nprint *\n";
    let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
    let options = model
        .zsh_options_at(source.find("print").unwrap())
        .expect("expected zsh options");

    assert_eq!(options.glob, OptionValue::Off);
}

#[test]
fn zsh_option_analysis_merges_conditionals_to_unknown_on_divergence() {
    let source = "if test \"$x\" = y; then\n  setopt no_glob\nfi\nprint *\n";
    let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
    let options = model
        .zsh_options_at(source.find("print").unwrap())
        .expect("expected zsh options");

    assert_eq!(options.glob, OptionValue::Unknown);
}

#[test]
fn zsh_option_analysis_respects_local_options_in_functions() {
    let source = "\
fn() {
  setopt local_options no_glob
}
fn
print *
";
    let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
    let options = model
        .zsh_options_at(source.find("print").unwrap())
        .expect("expected zsh options");

    assert_eq!(options.glob, OptionValue::On);
}

#[test]
fn zsh_option_analysis_applies_top_level_local_options_to_function_leaks() {
    let source = "\
setopt localoptions
fn() {
  setopt no_glob
}
fn
print *
";
    let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
    let options = model
        .zsh_options_at(source.find("print").unwrap())
        .expect("expected zsh options");

    assert_eq!(options.glob, OptionValue::On);
}

#[test]
fn zsh_option_analysis_leaks_function_option_updates_by_default() {
    let source = "\
fn() {
  setopt sh_word_split
}
fn
print $name
";
    let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
    assert!(
        model.scopes[0]
            .bindings
            .keys()
            .any(|name| name.as_str() == "fn"),
        "expected top-level function binding for `fn`"
    );
    assert!(
        model.recorded_program.function_body_scopes.len() == 1,
        "expected one recorded function body scope"
    );
    assert!(
        model
            .recorded_program
            .command_infos
            .values()
            .any(|info| info.static_callee.as_deref() == Some("fn")),
        "expected a static callee for the function call"
    );
    let options = model
        .zsh_options_at(source.find("print").unwrap())
        .expect("expected zsh options");

    assert_eq!(options.sh_word_split, OptionValue::On);
}

#[test]
fn zsh_option_analysis_falls_back_to_ancestor_state_in_uncalled_function_bodies() {
    let source = "\
fn() {
  print $name
}
";
    let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
    let options = model
        .zsh_options_at(source.find("print").unwrap())
        .expect("expected inherited zsh options");

    assert_eq!(options.sh_word_split, OptionValue::Off);
    assert_eq!(options.glob, OptionValue::On);
}

#[test]
fn zsh_option_analysis_merges_function_snapshots_from_multiple_call_contexts() {
    let source = "\
fn() {
  print $name
}
fn
setopt sh_word_split
fn
";
    let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
    let options = model
        .zsh_options_at(source.find("print").unwrap())
        .expect("expected merged function zsh options");

    assert_eq!(options.sh_word_split, OptionValue::Unknown);
}

#[test]
fn zsh_option_analysis_tracks_wrapped_option_builtins() {
    let source = "\
command setopt no_glob
builtin unsetopt short_loops
print *
";
    let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
    let options = model
        .zsh_options_at(source.find("print").unwrap())
        .expect("expected wrapped zsh option effects");

    assert_eq!(options.glob, OptionValue::Off);
    assert_eq!(options.short_loops, OptionValue::Off);
}

#[test]
fn zsh_option_analysis_skips_assignment_prefixes_after_wrappers() {
    for source in [
        "\
command FOO=1 setopt no_glob
print *
",
        "\
noglob FOO=1 setopt no_glob
print *
",
    ] {
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected wrapped zsh option effects");

        assert_eq!(options.glob, OptionValue::Off, "{source}");
    }
}

#[test]
fn zsh_option_analysis_tracks_command_repeated_p_wrapper() {
    let source = "\
command -pp setopt no_glob
print *
";
    let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
    let options = model
        .zsh_options_at(source.find("print").unwrap())
        .expect("expected wrapped zsh option effects");

    assert_eq!(options.glob, OptionValue::Off);
}

#[test]
fn zsh_option_analysis_tracks_exec_bundled_option_wrappers() {
    for source in [
        "\
exec -cl setopt no_glob
print *
",
        "\
exec -lc setopt no_glob
print *
",
        "\
exec -la shuck setopt no_glob
print *
",
    ] {
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected wrapped zsh option effects");

        assert_eq!(options.glob, OptionValue::Off, "{source}");
    }
}

#[test]
fn zsh_option_analysis_ignores_external_command_wrappers() {
    for source in [
        "\
sudo setopt no_glob
print *
",
        "\
find . -exec setopt no_glob \\;
print *
",
        "\
command FOO=1 sudo setopt no_glob
print *
",
        "\
command FOO=1 find . -exec setopt no_glob \\;
print *
",
    ] {
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected wrapped zsh options");

        assert_eq!(options.glob, OptionValue::On, "{source}");
    }
}

#[test]
fn zsh_option_analysis_ignores_command_lookup_modes() {
    for source in [
        "\
command -v setopt no_glob
print *
",
        "\
command -pv setopt no_glob
print *
",
        "\
command -pV setopt no_glob
print *
",
    ] {
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected wrapped zsh options");

        assert_eq!(options.glob, OptionValue::On, "{source}");
    }
}

#[test]
fn zsh_option_analysis_ignores_unsupported_precommand_options() {
    for source in [
        "\
command -x setopt no_glob
print *
",
        "\
builtin -x setopt no_glob
print *
",
        "\
exec -x setopt no_glob
print *
",
    ] {
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let options = model
            .zsh_options_at(source.find("print").unwrap())
            .expect("expected wrapped zsh options");

        assert_eq!(options.glob, OptionValue::On, "{source}");
    }
}
