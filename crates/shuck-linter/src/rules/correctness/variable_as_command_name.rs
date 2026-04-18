use rustc_hash::FxHashSet;
use shuck_ast::{Command, DeclOperand, Span};
use shuck_semantic::{BindingId, BindingKind, Reference, ReferenceKind};

use crate::facts::CommandId;
use crate::{Checker, ExpansionContext, Rule, Violation, WordFactContext};

pub struct VariableAsCommandName;

impl Violation for VariableAsCommandName {
    fn rule() -> Rule {
        Rule::VariableAsCommandName
    }

    fn message(&self) -> String {
        "unquoted expansion will not honor quotes or escapes stored in this variable".to_owned()
    }
}

pub fn variable_as_command_name(checker: &mut Checker) {
    let references = checker.semantic().references();
    let mut reference_indices = references
        .iter()
        .enumerate()
        .filter(|(_, reference)| {
            !matches!(
                reference.kind,
                ReferenceKind::DeclarationName | ReferenceKind::ImplicitRead
            )
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    reference_indices.sort_unstable_by_key(|&index| references[index].span.start.offset);

    let unsafe_bindings = unsafe_shell_quoting_bindings(checker, references, &reference_indices);
    if unsafe_bindings.is_empty() {
        return;
    }

    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .filter_map(|fact| {
            let context = fact.expansion_context()?;
            if !matches_sc2090_context(context) {
                return None;
            }
            if context != ExpansionContext::CommandName
                && command_is_eval(checker, fact.command_id())
            {
                return None;
            }

            Some(
                fact.unquoted_scalar_expansion_spans()
                    .iter()
                    .copied()
                    .filter(|span| {
                        expansion_span_uses_unsafe_binding(
                            *span,
                            checker,
                            references,
                            &reference_indices,
                            &unsafe_bindings,
                        )
                    }),
            )
        })
        .flatten()
        .collect::<Vec<_>>();
    let mut spans = spans;
    spans.extend(export_name_spans(checker, &unsafe_bindings));

    checker.report_all_dedup(spans, || VariableAsCommandName);
}

fn unsafe_shell_quoting_bindings(
    checker: &Checker<'_>,
    references: &[Reference],
    reference_indices: &[usize],
) -> FxHashSet<BindingId> {
    let scalar_bindings = checker
        .semantic()
        .bindings()
        .iter()
        .filter_map(|binding| {
            let context = binding_assignment_context(binding.kind)?;
            let word = checker.facts().scalar_binding_value(binding.span)?;
            Some((binding.id, word.span, context))
        })
        .collect::<Vec<_>>();

    let mut unsafe_bindings = scalar_bindings
        .iter()
        .filter_map(|(binding_id, word_span, context)| {
            let fact = checker.facts().word_fact(*word_span, *context)?;
            fact.contains_shell_quoting_literals()
                .then_some(*binding_id)
        })
        .collect::<FxHashSet<_>>();

    loop {
        let mut changed = false;
        for (binding_id, word_span, context) in &scalar_bindings {
            if unsafe_bindings.contains(binding_id) {
                continue;
            }
            let Some(fact) = checker.facts().word_fact(*word_span, *context) else {
                continue;
            };
            if fact.scalar_expansion_spans().iter().copied().any(|span| {
                plain_expansion_span_uses_unsafe_binding(
                    span,
                    checker,
                    references,
                    reference_indices,
                    &unsafe_bindings,
                )
            }) {
                unsafe_bindings.insert(*binding_id);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    unsafe_bindings
}

fn binding_assignment_context(kind: BindingKind) -> Option<WordFactContext> {
    match kind {
        BindingKind::Assignment | BindingKind::AppendAssignment => Some(
            WordFactContext::Expansion(ExpansionContext::AssignmentValue),
        ),
        BindingKind::Declaration(_) => Some(WordFactContext::Expansion(
            ExpansionContext::DeclarationAssignmentValue,
        )),
        BindingKind::ParameterDefaultAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::FunctionDefinition
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment
        | BindingKind::Nameref
        | BindingKind::Imported => None,
    }
}

fn matches_sc2090_context(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::RedirectTarget(_)
            | ExpansionContext::HereString
    )
}

fn command_is_eval(checker: &Checker<'_>, command_id: CommandId) -> bool {
    checker
        .facts()
        .command(command_id)
        .effective_or_literal_name()
        == Some("eval")
}

fn export_name_spans(checker: &Checker<'_>, unsafe_bindings: &FxHashSet<BindingId>) -> Vec<Span> {
    checker
        .facts()
        .commands()
        .iter()
        .flat_map(|command| {
            let Command::Decl(clause) = command.command() else {
                return Vec::new();
            };
            if clause.variant.as_str() != "export" {
                return Vec::new();
            }

            clause
                .operands
                .iter()
                .filter_map(|operand| {
                    let DeclOperand::Name(reference) = operand else {
                        return None;
                    };
                    checker
                        .semantic()
                        .visible_binding(&reference.name, reference.span)
                        .filter(|binding| unsafe_bindings.contains(&binding.id))
                        .map(|_| reference.span)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn expansion_span_uses_unsafe_binding(
    expansion_span: Span,
    checker: &Checker<'_>,
    references: &[Reference],
    reference_indices: &[usize],
    unsafe_bindings: &FxHashSet<BindingId>,
) -> bool {
    let first_reference = reference_indices.partition_point(|&index| {
        references[index].span.start.offset < expansion_span.start.offset
    });

    for &index in &reference_indices[first_reference..] {
        let reference = &references[index];
        if reference.span.start.offset > expansion_span.end.offset {
            break;
        }
        if !contains_span(expansion_span, reference.span) {
            continue;
        }
        if checker
            .semantic()
            .resolved_binding(reference.id)
            .is_some_and(|binding| unsafe_bindings.contains(&binding.id))
        {
            return true;
        }
    }

    false
}

fn plain_expansion_span_uses_unsafe_binding(
    expansion_span: Span,
    checker: &Checker<'_>,
    references: &[Reference],
    reference_indices: &[usize],
    unsafe_bindings: &FxHashSet<BindingId>,
) -> bool {
    let first_reference = reference_indices.partition_point(|&index| {
        references[index].span.start.offset < expansion_span.start.offset
    });

    for &index in &reference_indices[first_reference..] {
        let reference = &references[index];
        if reference.span.start.offset > expansion_span.end.offset {
            break;
        }
        if !contains_span(expansion_span, reference.span)
            || !expansion_span_is_plain_reference(expansion_span, reference, checker.source())
        {
            continue;
        }
        if checker
            .semantic()
            .resolved_binding(reference.id)
            .is_some_and(|binding| unsafe_bindings.contains(&binding.id))
        {
            return true;
        }
    }

    false
}

fn expansion_span_is_plain_reference(
    expansion_span: Span,
    reference: &Reference,
    source: &str,
) -> bool {
    let text = expansion_span.slice(source);
    text == format!("${}", reference.name.as_str())
        || text == format!("${{{}}}", reference.name.as_str())
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_argument_uses_of_shell_encoded_values() {
        let source = "\
#!/bin/sh
args='--name \"hello world\"'
printf '%s\n' $args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$args");
    }

    #[test]
    fn reports_command_names_here_strings_and_composite_words() {
        let source = "\
#!/bin/bash
cmd='printf \"hello world\"'
args='--name \"hello world\"'
$cmd
printf '%s\n' foo${args}bar
cat <<< $args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        let spans = diagnostics
            .iter()
            .map(|diagnostic| diagnostic.span.slice(source).to_owned())
            .collect::<Vec<_>>();
        assert_eq!(spans, vec!["$cmd", "${args}", "$args"]);
    }

    #[test]
    fn propagates_shell_encoded_values_through_intermediate_scalars() {
        let source = "\
#!/bin/bash
toolchain=\"--llvm-targets-to-build='X86;ARM;AArch64'\"
build_flags=\"$toolchain --install-prefix=/tmp\"
printf '%s\n' $build_flags
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$build_flags");
    }

    #[test]
    fn ignores_safe_quoted_and_eval_uses() {
        let source = "\
#!/bin/bash
cmd=printf
args='--name \"hello world\"'
$cmd '%s\n' ok
printf '%s\n' \"$args\"
cat <<< \"$args\"
eval printf '%s\n' $args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_exporting_shell_encoded_values() {
        let source = "\
#!/bin/sh
args='--name \"hello world\"'
export args
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "args");
    }

    #[test]
    fn does_not_propagate_through_substring_transformations() {
        let source = "\
#!/bin/bash
style=\"\\`'\"
quote=\"${style:1:1}\"\n\
export quote
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
