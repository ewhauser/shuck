use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Command, DeclOperand, Name, Span, Word, static_word_text};
use shuck_semantic::{
    Binding, BindingAttributes, BindingId, BindingKind, Reference, ReferenceKind,
};

use crate::facts::{CommandId, word_spans};
use crate::{Checker, ExpansionContext, SimpleTestShape, SimpleTestSyntax, WordFactContext};

pub(crate) struct ShellQuotingReuseAnalysis {
    pub assignment_spans: Vec<Span>,
    pub use_spans: Vec<Span>,
}

pub(crate) fn analyze_shell_quoting_reuse(checker: &Checker<'_>) -> ShellQuotingReuseAnalysis {
    let scalar_bindings = checker
        .semantic()
        .bindings()
        .iter()
        .filter_map(|binding| {
            let context = binding_assignment_context(binding.kind)?;
            let word = checker.facts().binding_value(binding.id)?.scalar_word()?;
            Some(ScalarBinding {
                id: binding.id,
                word,
                context,
            })
        })
        .collect::<Vec<_>>();
    let scalar_binding_map = scalar_bindings
        .iter()
        .copied()
        .map(|binding| (binding.id, binding))
        .collect::<FxHashMap<_, _>>();

    let direct_unsafe_bindings = scalar_bindings
        .iter()
        .filter_map(|binding| {
            let fact = checker
                .facts()
                .word_fact(binding.word.span, binding.context)?;
            fact.contains_shell_quoting_literals().then_some(binding.id)
        })
        .collect::<FxHashSet<_>>();
    if direct_unsafe_bindings.is_empty() {
        return ShellQuotingReuseAnalysis {
            assignment_spans: Vec::new(),
            use_spans: Vec::new(),
        };
    }

    let dependency_map = scalar_bindings
        .iter()
        .map(|binding| {
            (
                binding.id,
                plain_scalar_reference_bindings(binding.word.span, checker),
            )
        })
        .collect::<FxHashMap<_, _>>();

    let mut root_cache = FxHashMap::<BindingId, FxHashSet<BindingId>>::default();
    let mut used_root_bindings = FxHashSet::default();
    let mut use_spans = Vec::new();
    for fact in checker.facts().word_facts() {
        let Some(context) = fact.expansion_context() else {
            continue;
        };
        if !matches_sc2090_context(context) {
            continue;
        }
        if context != ExpansionContext::CommandName && command_is_eval(checker, fact.command_id()) {
            continue;
        }

        for span in fact.unquoted_scalar_expansion_spans().iter().copied() {
            let roots = root_bindings_for_expansion_span(
                span,
                checker,
                &direct_unsafe_bindings,
                &dependency_map,
                &mut root_cache,
            );
            if roots.is_empty() {
                continue;
            }

            used_root_bindings.extend(roots);
            use_spans.push(span);
        }
    }

    use_spans.extend(export_name_spans(
        checker,
        &direct_unsafe_bindings,
        &dependency_map,
        &mut root_cache,
        &mut used_root_bindings,
    ));
    use_spans.extend(bracket_v_name_spans(
        checker,
        &direct_unsafe_bindings,
        &dependency_map,
        &mut root_cache,
        &mut used_root_bindings,
    ));
    used_root_bindings.extend(export_assignment_root_bindings(
        checker,
        &direct_unsafe_bindings,
        &dependency_map,
        &mut root_cache,
    ));

    sort_and_dedup_spans(&mut use_spans);

    let mut assignment_spans = used_root_bindings
        .iter()
        .filter_map(|binding_id| scalar_binding_map.get(binding_id).copied())
        .map(|binding| assignment_value_report_span(binding, checker))
        .collect::<Vec<_>>();
    sort_and_dedup_spans(&mut assignment_spans);

    ShellQuotingReuseAnalysis {
        assignment_spans,
        use_spans,
    }
}

#[derive(Clone, Copy)]
struct ScalarBinding<'a> {
    id: BindingId,
    word: &'a Word,
    context: WordFactContext,
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

fn plain_scalar_reference_bindings(word_span: Span, checker: &Checker<'_>) -> Vec<BindingId> {
    let Some(fact) = checker.facts().any_word_fact(word_span) else {
        return Vec::new();
    };

    let bindings = fact
        .scalar_expansion_spans()
        .iter()
        .copied()
        .flat_map(|span| direct_reference_bindings_in_span(span, checker, true))
        .collect::<Vec<_>>();
    dedup_binding_ids(bindings)
}

fn root_bindings_for_expansion_span(
    expansion_span: Span,
    checker: &Checker<'_>,
    direct_unsafe_bindings: &FxHashSet<BindingId>,
    dependency_map: &FxHashMap<BindingId, Vec<BindingId>>,
    root_cache: &mut FxHashMap<BindingId, FxHashSet<BindingId>>,
) -> FxHashSet<BindingId> {
    let mut roots = FxHashSet::default();
    for binding_id in direct_reference_bindings_in_span(expansion_span, checker, false) {
        roots.extend(root_bindings_for_binding(
            binding_id,
            direct_unsafe_bindings,
            dependency_map,
            root_cache,
            &mut FxHashSet::default(),
        ));
    }
    roots
}

fn root_bindings_for_binding(
    binding_id: BindingId,
    direct_unsafe_bindings: &FxHashSet<BindingId>,
    dependency_map: &FxHashMap<BindingId, Vec<BindingId>>,
    root_cache: &mut FxHashMap<BindingId, FxHashSet<BindingId>>,
    visiting: &mut FxHashSet<BindingId>,
) -> FxHashSet<BindingId> {
    if let Some(cached) = root_cache.get(&binding_id) {
        return cached.clone();
    }
    if !visiting.insert(binding_id) {
        return FxHashSet::default();
    }

    let mut roots = FxHashSet::default();
    if direct_unsafe_bindings.contains(&binding_id) {
        roots.insert(binding_id);
    }
    if let Some(dependencies) = dependency_map.get(&binding_id) {
        for dependency in dependencies {
            roots.extend(root_bindings_for_binding(
                *dependency,
                direct_unsafe_bindings,
                dependency_map,
                root_cache,
                visiting,
            ));
        }
    }

    visiting.remove(&binding_id);
    root_cache.insert(binding_id, roots.clone());
    roots
}

fn direct_reference_bindings_in_span(
    expansion_span: Span,
    checker: &Checker<'_>,
    require_plain_reference: bool,
) -> Vec<BindingId> {
    let mut bindings = Vec::new();
    for reference in checker.semantic().references_in_span(expansion_span) {
        if matches!(
            reference.kind,
            ReferenceKind::DeclarationName | ReferenceKind::ImplicitRead
        ) || (require_plain_reference
            && !expansion_span_is_plain_reference(expansion_span, reference, checker.source()))
        {
            continue;
        }
        if let Some(binding) = checker.semantic().resolved_binding(reference.id) {
            bindings.push(binding.id);
        }
    }
    dedup_binding_ids(bindings)
}

fn export_name_spans(
    checker: &Checker<'_>,
    direct_unsafe_bindings: &FxHashSet<BindingId>,
    dependency_map: &FxHashMap<BindingId, Vec<BindingId>>,
    root_cache: &mut FxHashMap<BindingId, FxHashSet<BindingId>>,
    used_root_bindings: &mut FxHashSet<BindingId>,
) -> Vec<Span> {
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
                    let binding = checker
                        .semantic()
                        .visible_binding(&reference.name, reference.span)?;
                    let roots = root_bindings_for_binding(
                        binding.id,
                        direct_unsafe_bindings,
                        dependency_map,
                        root_cache,
                        &mut FxHashSet::default(),
                    );
                    if roots.is_empty() {
                        return None;
                    }

                    used_root_bindings.extend(roots);
                    Some(reference.span)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn bracket_v_name_spans(
    checker: &Checker<'_>,
    direct_unsafe_bindings: &FxHashSet<BindingId>,
    dependency_map: &FxHashMap<BindingId, Vec<BindingId>>,
    root_cache: &mut FxHashMap<BindingId, FxHashSet<BindingId>>,
    used_root_bindings: &mut FxHashSet<BindingId>,
) -> Vec<Span> {
    checker
        .facts()
        .commands()
        .iter()
        .filter_map(|command| {
            let simple_test = command.simple_test()?;
            if simple_test.syntax() != SimpleTestSyntax::Bracket
                || simple_test.effective_shape() != SimpleTestShape::Unary
            {
                return None;
            }

            let operator = simple_test
                .effective_operator_word()
                .and_then(|word| static_word_text(word, checker.source()));
            if operator.as_deref() != Some("-v") {
                return None;
            }

            let operand = simple_test.effective_operands().get(1)?;
            let name = static_word_text(operand, checker.source())?;
            let binding_id = checker
                .semantic()
                .bindings_for(&Name::from(name.as_ref()))
                .iter()
                .copied()
                .filter(|binding_id| {
                    let binding = checker.semantic().binding(*binding_id);
                    binding.span.start.offset <= operand.span.start.offset
                        && is_test_v_variable_binding(binding)
                })
                .max_by_key(|binding_id| {
                    checker.semantic().binding(*binding_id).span.start.offset
                })?;
            let roots = root_bindings_for_binding(
                binding_id,
                direct_unsafe_bindings,
                dependency_map,
                root_cache,
                &mut FxHashSet::default(),
            );
            if roots.is_empty() {
                return None;
            }

            used_root_bindings.extend(roots);
            Some(operand.span)
        })
        .collect()
}

fn is_test_v_variable_binding(binding: &Binding) -> bool {
    match binding.kind {
        BindingKind::FunctionDefinition => false,
        BindingKind::Imported => !binding
            .attributes
            .contains(BindingAttributes::IMPORTED_FUNCTION),
        BindingKind::Assignment
        | BindingKind::ParameterDefaultAssignment
        | BindingKind::AppendAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::Declaration(_)
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment
        | BindingKind::Nameref => true,
    }
}

fn export_assignment_root_bindings(
    checker: &Checker<'_>,
    direct_unsafe_bindings: &FxHashSet<BindingId>,
    dependency_map: &FxHashMap<BindingId, Vec<BindingId>>,
    root_cache: &mut FxHashMap<BindingId, FxHashSet<BindingId>>,
) -> FxHashSet<BindingId> {
    let repeated_targets = repeated_export_assignment_targets(checker);
    if repeated_targets.is_empty() {
        return FxHashSet::default();
    }

    let mut roots = FxHashSet::default();
    for command in checker.facts().commands() {
        let Command::Decl(clause) = command.command() else {
            continue;
        };
        if clause.variant.as_str() != "export" {
            continue;
        }

        for operand in &clause.operands {
            let DeclOperand::Assignment(assignment) = operand else {
                continue;
            };
            if !repeated_targets.contains(assignment.target.name.as_str()) {
                continue;
            }
            let shuck_ast::AssignmentValue::Scalar(word) = &assignment.value else {
                continue;
            };

            for binding_id in plain_scalar_reference_bindings(word.span, checker) {
                roots.extend(root_bindings_for_binding(
                    binding_id,
                    direct_unsafe_bindings,
                    dependency_map,
                    root_cache,
                    &mut FxHashSet::default(),
                ));
            }
        }
    }

    roots
}

fn repeated_export_assignment_targets(checker: &Checker<'_>) -> FxHashSet<String> {
    let mut counts = FxHashMap::<String, usize>::default();
    for command in checker.facts().commands() {
        let Command::Decl(clause) = command.command() else {
            continue;
        };
        if clause.variant.as_str() != "export" {
            continue;
        }

        for operand in &clause.operands {
            let DeclOperand::Assignment(assignment) = operand else {
                continue;
            };
            *counts
                .entry(assignment.target.name.as_str().to_owned())
                .or_default() += 1;
        }
    }

    counts
        .into_iter()
        .filter_map(|(name, count)| (count > 1).then_some(name))
        .collect()
}

fn assignment_value_report_span(binding: ScalarBinding<'_>, checker: &Checker<'_>) -> Span {
    word_spans::word_shell_quoting_literal_run_span_in_source(binding.word, checker.source())
        .unwrap_or(binding.word.span)
}

fn expansion_span_is_plain_reference(
    expansion_span: Span,
    reference: &Reference,
    source: &str,
) -> bool {
    let text = expansion_span.slice(source).as_bytes();
    let name = reference.name.as_str().as_bytes();
    let plain = text.len() == name.len() + 1 && text.first() == Some(&b'$') && &text[1..] == name;
    let braced = text.len() == name.len() + 3
        && text.starts_with(b"${")
        && text.ends_with(b"}")
        && &text[2..text.len() - 1] == name;
    plain || braced
}

fn sort_and_dedup_spans(spans: &mut Vec<Span>) {
    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
}

fn dedup_binding_ids(bindings: Vec<BindingId>) -> Vec<BindingId> {
    let mut seen = FxHashSet::default();
    bindings
        .into_iter()
        .filter(|binding_id| seen.insert(*binding_id))
        .collect()
}
