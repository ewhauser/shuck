use rustc_hash::FxHashSet;
use shuck_ast::{Command, Span};
use shuck_semantic::{
    Binding, BindingAttributes, BindingId, BindingKind, DeclarationBuiltin, DeclarationOperand,
    Reference,
};

use crate::{Checker, LinterFacts, Rule, Violation, WordQuote};

pub struct QuotedBashSource;

impl Violation for QuotedBashSource {
    fn rule() -> Rule {
        Rule::QuotedBashSource
    }

    fn message(&self) -> String {
        "array references should choose an explicit element or selector".to_owned()
    }
}

pub fn quoted_bash_source(checker: &mut Checker) {
    let semantic = checker.semantic();
    let analysis = semantic.analysis();
    let candidate_spans = checker
        .facts()
        .plain_unindexed_reference_spans()
        .iter()
        .copied()
        .map(span_key)
        .collect::<FxHashSet<_>>();
    let spans = semantic
        .references()
        .iter()
        .filter(|reference| candidate_spans.contains(&span_key(reference.span)))
        .filter(|reference| {
            reference_is_array_like(checker.facts(), semantic, &analysis, reference)
        })
        .map(|reference| reference.span)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || QuotedBashSource);
}

fn span_is_within(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn span_key(span: Span) -> (usize, usize) {
    (span.start.offset, span.end.offset)
}

fn reference_is_array_like(
    facts: &LinterFacts<'_>,
    semantic: &shuck_semantic::SemanticModel,
    analysis: &shuck_semantic::SemanticAnalysis<'_>,
    reference: &Reference,
) -> bool {
    if semantic.is_guarded_parameter_reference(reference.id)
        || reference_has_prior_presence_test(facts, semantic, reference)
        || reference_reads_into_same_name_array_writer(facts, semantic, reference)
    {
        return false;
    }
    if let Some(binding) = semantic.resolved_binding(reference.id)
        && semantic.binding_visible_at(binding.id, reference.span)
        && !binding_is_array_like(binding)
        && !binding_inherits_indexed_array_type(semantic, analysis, binding)
        && (binding_resets_indexed_array_type(binding)
            || binding_has_prior_local_barrier(semantic, binding))
    {
        return false;
    }

    if is_bash_runtime_array_name(reference.name.as_str()) {
        return true;
    }

    let mut binding_ids = Vec::new();
    let mut seen = FxHashSet::default();
    if let Some(binding) = semantic.resolved_binding(reference.id)
        && !binding_is_array_like(binding)
        && seen.insert(binding.id)
    {
        binding_ids.push(binding.id);
    }
    for binding_id in candidate_binding_ids_for_reference(semantic, analysis, reference) {
        if seen.insert(binding_id) {
            binding_ids.push(binding_id);
        }
    }

    binding_ids.into_iter().any(|binding_id| {
        let binding = semantic.binding(binding_id);
        !binding_reset_by_name_only_declaration_before(semantic, binding, reference.span)
            && (binding_is_array_like(binding)
                || binding_inherits_indexed_array_type(semantic, analysis, binding))
    })
}

fn binding_is_array_like(binding: &Binding) -> bool {
    let declared_array = binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC);
    (declared_array && !is_uninitialized_local_array_declaration(binding))
        || matches!(
            binding.kind,
            BindingKind::ArrayAssignment | BindingKind::MapfileTarget
        )
}

fn binding_inherits_indexed_array_type(
    semantic: &shuck_semantic::SemanticModel,
    analysis: &shuck_semantic::SemanticAnalysis<'_>,
    binding: &Binding,
) -> bool {
    if binding_resets_indexed_array_type(binding) {
        return false;
    }

    let initialized_scalar_declaration = matches!(binding.kind, BindingKind::Declaration(_))
        && binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
        && !binding
            .attributes
            .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC);
    let append_declaration = binding_is_append_declaration(semantic, binding);
    let prior_local_barrier = binding_has_prior_local_barrier(semantic, binding);
    let reaching_bindings = analysis
        .reaching_bindings_for_name(&binding.name, binding.span)
        .into_iter()
        .collect::<FxHashSet<_>>();
    let informative_same_scope_reaching = reaching_bindings
        .iter()
        .any(|candidate_id| *candidate_id != binding.id);
    let prior_bindings = semantic
        .bindings_for(&binding.name)
        .iter()
        .copied()
        .filter(|candidate_id| {
            let candidate = semantic.binding(*candidate_id);
            let same_scope_candidate_allowed =
                if initialized_scalar_declaration && !append_declaration {
                    false
                } else {
                    append_declaration
                        || !informative_same_scope_reaching
                        || reaching_bindings.contains(candidate_id)
                };
            candidate.span.start.offset < binding.span.start.offset
                && ((candidate.scope != binding.scope && !prior_local_barrier)
                    || same_scope_candidate_allowed)
                && !binding_reset_by_name_only_declaration_before(semantic, candidate, binding.span)
        })
        .map(|candidate_id| semantic.binding(candidate_id));

    for candidate in prior_bindings.rev() {
        if binding_resets_indexed_array_type(candidate) {
            return false;
        }
        if binding_is_sticky_indexed_array(candidate) {
            return true;
        }
    }

    false
}

fn binding_resets_indexed_array_type(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::ArithmeticAssignment
            | BindingKind::GetoptsTarget
            | BindingKind::Imported
            | BindingKind::LoopVariable
            | BindingKind::PrintfTarget
    ) || (matches!(binding.kind, BindingKind::ReadTarget)
        && !binding.attributes.contains(BindingAttributes::ARRAY))
        || (matches!(binding.kind, BindingKind::Declaration(_))
            && !binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED)
            && !binding
                .attributes
                .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC))
}

fn binding_is_sticky_indexed_array(binding: &Binding) -> bool {
    !is_uninitialized_local_array_declaration(binding)
        && (binding.attributes.contains(BindingAttributes::ARRAY)
            || matches!(
                binding.kind,
                BindingKind::ArrayAssignment | BindingKind::MapfileTarget
            ))
}

fn is_uninitialized_local_array_declaration(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::Declaration(DeclarationBuiltin::Local)
    ) && binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        && !binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED)
}

fn reference_reads_into_same_name_array_writer(
    facts: &LinterFacts<'_>,
    semantic: &shuck_semantic::SemanticModel,
    reference: &Reference,
) -> bool {
    semantic
        .bindings_for(&reference.name)
        .iter()
        .copied()
        .any(|binding_id| {
            let binding = semantic.binding(binding_id);
            binding.span.start.offset <= reference.span.start.offset
                && same_simple_command_is_assignment_only(facts, binding.span, reference.span)
                    .is_some_and(|assignment_only| {
                        binding_suppresses_same_command_array_read(binding, assignment_only)
                    })
        })
}

fn reference_has_prior_presence_test(
    facts: &LinterFacts<'_>,
    semantic: &shuck_semantic::SemanticModel,
    reference: &Reference,
) -> bool {
    if loop_header_word_quote(facts, reference.span)
        .is_some_and(|quote| quote != WordQuote::Unquoted)
    {
        return false;
    }

    let reference_binding = semantic
        .resolved_binding(reference.id)
        .map(|binding| binding.id);

    facts
        .presence_test_references(&reference.name)
        .iter()
        .any(|test| {
            test.command_span().end.offset < reference.span.start.offset
                && semantic
                    .resolved_binding(test.reference_id())
                    .map(|binding| binding.id)
                    == reference_binding
        })
        || facts
            .presence_test_names(&reference.name)
            .iter()
            .any(|test| {
                test.command_span().end.offset < reference.span.start.offset
                    && presence_test_name_binding(semantic, &reference.name, test.tested_span())
                        == reference_binding
            })
}

fn loop_header_word_quote(facts: &LinterFacts<'_>, span: Span) -> Option<WordQuote> {
    facts
        .for_headers()
        .iter()
        .flat_map(|header| header.words().iter())
        .chain(
            facts
                .select_headers()
                .iter()
                .flat_map(|header| header.words().iter()),
        )
        .find(|word| span_is_within(word.span(), span))
        .map(|word| word.classification().quote)
}

fn binding_suppresses_same_command_array_read(binding: &Binding, assignment_only: bool) -> bool {
    matches!(binding.kind, BindingKind::MapfileTarget)
        || (matches!(binding.kind, BindingKind::ReadTarget)
            && binding.attributes.contains(BindingAttributes::ARRAY))
        || (matches!(binding.kind, BindingKind::ArrayAssignment) && assignment_only)
}

fn presence_test_name_binding(
    semantic: &shuck_semantic::SemanticModel,
    name: &shuck_ast::Name,
    tested_span: Span,
) -> Option<BindingId> {
    semantic
        .bindings_for(name)
        .iter()
        .copied()
        .rev()
        .find(|binding_id| semantic.binding_visible_at(*binding_id, tested_span))
}

fn same_simple_command_is_assignment_only(
    facts: &LinterFacts<'_>,
    binding_span: Span,
    reference_span: Span,
) -> Option<bool> {
    facts
        .commands()
        .iter()
        .filter(|command| matches!(command.command(), Command::Simple(_)))
        .filter(|command| {
            let span = command.span();
            span_is_within(span, binding_span) && span_is_within(span, reference_span)
        })
        .min_by_key(|command| command.span().end.offset - command.span().start.offset)
        .map(|command| command.literal_name() == Some(""))
}

fn candidate_binding_ids_for_reference(
    semantic: &shuck_semantic::SemanticModel,
    analysis: &shuck_semantic::SemanticAnalysis<'_>,
    reference: &Reference,
) -> Vec<BindingId> {
    let all_bindings = semantic.bindings_for(&reference.name);
    let binding_ids = semantic
        .ancestor_scopes(reference.scope)
        .filter_map(|scope| {
            all_bindings.iter().copied().rev().find(|binding_id| {
                let binding = semantic.binding(*binding_id);
                binding.scope == scope && semantic.binding_visible_at(*binding_id, reference.span)
            })
        })
        .collect::<Vec<_>>();
    if !binding_ids.is_empty() {
        return binding_ids;
    }

    let binding_ids = analysis.reaching_bindings_for_name(&reference.name, reference.span);
    if !binding_ids.is_empty() {
        return binding_ids;
    }

    semantic
        .ancestor_scopes(reference.scope)
        .skip(1)
        .filter_map(|scope| {
            all_bindings.iter().copied().rev().find(|binding_id| {
                let binding = semantic.binding(*binding_id);
                binding.scope == scope && semantic.binding_visible_at(*binding_id, reference.span)
            })
        })
        .chain(all_bindings.iter().copied().filter(|binding_id| {
            let binding = semantic.binding(*binding_id);
            binding.scope != reference.scope
                && binding.span.start.offset < reference.span.start.offset
        }))
        .collect::<FxHashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
}

fn binding_reset_by_name_only_declaration_before(
    semantic: &shuck_semantic::SemanticModel,
    binding: &Binding,
    at: Span,
) -> bool {
    semantic.declarations().iter().any(|declaration| {
        matches!(declaration.builtin, DeclarationBuiltin::Local)
            && declaration.span.start.offset > binding.span.start.offset
            && declaration.span.end.offset < at.start.offset
            && semantic.scope_at(declaration.span.start.offset) == binding.scope
            && declaration.operands.iter().any(|operand| {
                matches!(
                    operand,
                    DeclarationOperand::Name { name, .. } if name == &binding.name
                )
            })
    })
}

fn binding_has_prior_local_barrier(
    semantic: &shuck_semantic::SemanticModel,
    binding: &Binding,
) -> bool {
    semantic.declarations().iter().any(|declaration| {
        matches!(declaration.builtin, DeclarationBuiltin::Local)
            && declaration.span.end.offset < binding.span.start.offset
            && semantic.scope_at(declaration.span.start.offset) == binding.scope
            && declaration.operands.iter().any(|operand| {
                matches!(
                    operand,
                    DeclarationOperand::Name { name, .. }
                        | DeclarationOperand::Assignment { name, .. }
                        if name == &binding.name
                )
            })
    })
}

fn binding_is_append_declaration(
    semantic: &shuck_semantic::SemanticModel,
    binding: &Binding,
) -> bool {
    semantic.declarations().iter().any(|declaration| {
        semantic.scope_at(declaration.span.start.offset) == binding.scope
            && declaration.operands.iter().any(|operand| {
                matches!(
                    operand,
                    DeclarationOperand::Assignment {
                        name,
                        name_span,
                        append: true,
                        ..
                    } if name == &binding.name && name_span == &binding.span
                )
            })
    })
}

fn is_bash_runtime_array_name(name: &str) -> bool {
    matches!(
        name,
        "BASH_ALIASES"
            | "BASH_ARGC"
            | "BASH_ARGV"
            | "BASH_CMDS"
            | "BASH_LINENO"
            | "BASH_REMATCH"
            | "BASH_SOURCE"
            | "BASH_VERSINFO"
            | "COMP_WORDS"
            | "COMPREPLY"
            | "COPROC"
            | "DIRSTACK"
            | "FUNCNAME"
            | "GROUPS"
            | "MAPFILE"
            | "PIPESTATUS"
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, lint_file_at_path};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn reports_plain_unindexed_array_references() {
        let source = "\
#!/bin/bash
arr=(one two)
declare -A map=([key]=value)
read -ra read_items
mapfile map_items
x=\"$BASH_SOURCE\"
y=\"${BASH_SOURCE}\"
printf '%s\\n' $arr \"${arr}\" pre${arr}post \"$map\" \"$read_items\" \"$map_items\"
source \"$(dirname \"$BASH_SOURCE\")/helper.bash\"
if [[ \"$BASH_SOURCE\" == foo ]]; then :; fi
for item in \"$BASH_SOURCE\"; do
  :
done
cat <<EOF
$arr
${arr}
EOF
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$BASH_SOURCE",
                "${BASH_SOURCE}",
                "$arr",
                "${arr}",
                "${arr}",
                "$map",
                "$read_items",
                "$map_items",
                "$BASH_SOURCE",
                "$BASH_SOURCE",
                "$BASH_SOURCE",
                "$arr",
                "${arr}",
            ]
        );
    }

    #[test]
    fn ignores_scalar_indexed_selector_and_non_access_forms() {
        let source = "\
#!/bin/bash
name=scalar
MAPFILE=scalar
arr=(one two)
x=$BASH_SOURCE
y=${BASH_SOURCE}
z=\"${BASH_SOURCE[0]}\"
q=\"${BASH_SOURCE[@]}\"
r=\"${BASH_SOURCE[*]}\"
s=\"${BASH_SOURCE%/*}\"
t=\"${BASH_SOURCE:-fallback}\"
v=\"${BASH_SOURCE-}\"
u=\"\\$BASH_SOURCE\"
printf '%s\\n' \"$name\" \"${arr[0]}\" \"${arr[@]}\" \"${arr[*]}\" \"${arr%one}\" \"${arr:-fallback}\"
only_declared() {
  local -a local_array
  printf '%s\\n' \"$local_array\"
}
for item in \"$@\"; do
  item=($item)
done
read -ra read_items <<<\"$read_items\"
printf '%s\\n' \"$MAPFILE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$BASH_SOURCE", "${BASH_SOURCE}", "$MAPFILE"]
        );
    }

    #[test]
    fn ignores_follow_up_loop_headers_after_presence_guard() {
        let source = "\
#!/bin/bash
filelist=()
filelist+=(\"$1\")
if [ -z \"${filelist[*]}\" ]; then
  exit
fi
for item in $filelist; do
  :
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_nested_follow_up_loop_headers_after_presence_guard() {
        let source = "\
#!/bin/bash
filelist=()
filelist+=(\"$1\")
if [ -z \"${filelist[*]}\" ]; then
  exit
fi
tests=\"$(for item in $filelist; do
  :
done)\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn string_binary_conditions_do_not_count_as_presence_guards() {
        let source = "\
#!/bin/bash
apt_pkgs=()
for pkg in \"$@\"; do
  pkg=(one two three)
  if [[ \"${pkg[0]}\" == one ]]; then
    :
  fi
  if hasPackage \"$pkg\"; then
    apt_pkgs+=(\"$pkg\")
  fi
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$pkg", "$pkg"]
        );
    }

    #[test]
    fn unset_does_not_reset_array_type() {
        let source = "\
#!/bin/bash
cleared_array=(one two)
unset cleared_array
cleared_array=scalar
printf '%s\\n' \"$cleared_array\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$cleared_array"]
        );
    }

    #[test]
    fn target_rebindings_reset_inherited_array_type() {
        let source = "\
#!/bin/bash
loop_value=(one two)
for loop_value in one two; do
  printf '%s\\n' \"$loop_value\"
done
read_value=(one two)
read read_value <<<input
printf '%s\\n' \"$read_value\"
printf_value=(one two)
printf -v printf_value '%s' input
printf '%s\\n' \"$printf_value\"
local_reset() {
  local local_value=(one two)
  local local_value
  printf '%s\\n' \"$local_value\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_unbound_runtime_arrays_without_bash_prelude() {
        let source = "\
#!/bin/sh
printf '%s\\n' \"$BASH_SOURCE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$BASH_SOURCE"]
        );
    }

    #[test]
    fn reports_runtime_array_names_even_after_scalar_rebinding() {
        let source = "\
#!/bin/bash
MAPFILE=scalar
printf '%s\\n' \"$MAPFILE\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$MAPFILE"]
        );
    }

    #[test]
    fn array_declarations_stay_sticky_through_plain_assignments() {
        let source = "\
#!/bin/bash
declare -a additional_packages
additional_packages=$1
split_string ${additional_packages}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${additional_packages}"]
        );
    }

    #[test]
    fn later_presence_guards_only_suppress_the_same_binding() {
        let source = "\
#!/bin/bash
foo=scalar
[ -n \"$foo\" ]
foo=(one two)
printf '%s\\n' \"$foo\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$foo"]
        );
    }

    #[test]
    fn variable_set_presence_guards_suppress_follow_up_refs() {
        let source = "\
#!/bin/bash
arr=(one two)
[[ -v arr ]]
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn variable_set_presence_guards_do_not_cross_rebindings() {
        let source = "\
#!/bin/bash
arr=scalar
[[ -v arr ]]
arr=(one two)
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn prior_presence_guards_in_sibling_case_arms_suppress_follow_up_refs() {
        let source = "\
#!/bin/bash
f() {
  local dir
  case \"$1\" in
    up) dir=(\"Up\");;
  esac
  case \"$2\" in
    hat)
      [[ -n \"$dir\" ]]
      ;;
    *)
      [[ \"$dir\" == \"Up\" || \"$dir\" == \"Left\" ]]
      ;;
  esac
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$dir"]
        );
    }

    #[test]
    fn attribute_only_declarations_keep_array_type() {
        let source = "\
#!/bin/bash
arr=(one two)
readonly arr
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn function_local_declare_arrays_still_warn() {
        let source = "\
#!/bin/bash
f() {
  declare -a items
  printf '%s\\n' \"$items\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$items"]
        );
    }

    #[test]
    fn nested_command_substitution_presence_tests_do_not_suppress_follow_up_refs() {
        let source = "\
#!/bin/bash
arr=(one two)
[ -n \"$(printf '%s' \"$arr\")\" ]
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr", "$arr"]
        );
    }

    #[test]
    fn presence_tests_inside_command_substitutions_suppress_later_refs() {
        let source = "\
#!/bin/bash
arr=(one two)
out=$( [ -n \"$arr\" ]; printf x )
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn same_command_prefix_array_assignments_still_warn() {
        let source = "\
#!/bin/bash
arr=(old1 old2)
arr=(new1 new2) printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn read_option_values_do_not_become_array_targets() {
        let source = "\
#!/bin/bash
delimiter=:
read -d delimiter -a arr <<<\":\"
printf '%s\\n' \"$delimiter\"
printf '%s\\n' \"$arr\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$arr"]
        );
    }

    #[test]
    fn mapfile_option_values_do_not_become_array_targets() {
        let source = "\
#!/bin/bash
callback=scalar
mapfile -C callback -c 1 lines < <(printf '%s\\n' value)
printf '%s\\n' \"$callback\"
printf '%s\\n' \"$lines\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$lines"]
        );
    }

    #[test]
    fn local_scalar_assignments_do_not_inherit_outer_array_bindings() {
        let source = "\
#!/bin/bash
declare -a ids
ids=()
set_to_liked() {
  local ids
  { local IFS=','; ids=\"$*\"; }
  if [ -z \"$ids\" ]; then
    return
  fi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn printf_targets_after_local_declarations_do_not_inherit_outer_arrays() {
        let source = "\
#!/bin/bash
args=(\"$@\")
f() {
  local args
  printf -v args '%q ' \"$@\"
  printf '%s\\n' \"$args\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn local_append_declarations_keep_array_type() {
        let source = "\
#!/bin/bash
f() {
  local DOKKU_LOGS_CMD=()
  DOKKU_LOGS_CMD+=\"(cmd)\"
  local DOKKU_LOGS_CMD+=\"; \"
  bash -c \"($DOKKU_LOGS_CMD)\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$DOKKU_LOGS_CMD"]
        );
    }

    #[test]
    fn ignores_references_inside_own_array_assignment() {
        let source = "\
#!/bin/bash
TERMUX_PKG_VERSION=(\"$(printf '%s\\n' \"$TERMUX_PKG_VERSION\")\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_references_inside_same_name_array_readers() {
        let source = "\
#!/bin/bash
read -r -a key_value <<<\"$(printf '%s\\n' \"$key_value\")\"
mapfile -t ports_configured < <(printf '%s\\n' \"${ports_configured}\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn imported_bindings_reset_inherited_array_type() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        fs::write(
            &main,
            "\
#!/bin/bash
TERMUX_PKG_VERSION=(\"$(. ./helper.sh; printf '%s\\n' \"$TERMUX_PKG_VERSION\")\")
",
        )
        .unwrap();
        fs::write(&helper, "TERMUX_PKG_VERSION=helper\n").unwrap();

        let source = fs::read_to_string(&main).unwrap();
        let output = Parser::new(&source).parse().unwrap();
        let indexer = Indexer::new(&source, &output);
        let diagnostics = lint_file_at_path(
            &output.file,
            &source,
            &indexer,
            &LinterSettings::for_rule(Rule::QuotedBashSource),
            None,
            Some(&main),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn follows_prior_visible_array_bindings() {
        let source = "\
#!/bin/bash
before_use() {
  printf '%s\\n' \"$future_array\"
}
future_array=(one two)
after_use() {
  printf '%s\\n' \"$future_array\"
}
former_array=(one two)
former_array=scalar
printf '%s\\n' \"$former_array\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$future_array", "$former_array"]
        );
    }

    #[test]
    fn follows_prior_array_bindings_by_source_order() {
        let source = "\
#!/bin/bash
first_function() {
  target=(one two)
}
second_function() {
  local target=$1
  printf '%s\\n' \"$target\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$target"]
        );
    }

    #[test]
    fn reports_runtime_arrays_inside_assign_default_and_error_operands() {
        let source = "\
#!/bin/bash
: ${PROG:=$(basename ${BASH_SOURCE})}
local PATTERN=${2:?$FUNCNAME: a pattern is required}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::QuotedBashSource));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${BASH_SOURCE}", "$FUNCNAME"]
        );
    }
}
