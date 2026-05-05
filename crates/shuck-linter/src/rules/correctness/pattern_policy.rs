use shuck_ast::{Span, Word, static_word_text, word_span_is_plain_parameter_reference};
use shuck_semantic::{BindingId, Reference, ReferenceKind};

use crate::{Checker, ShellDialect};

pub(crate) fn word_expands_only_static_pattern_safe_literals(
    checker: &Checker<'_>,
    word: &Word,
) -> bool {
    span_expands_only_static_pattern_safe_literals(
        checker,
        word.span,
        word_span_is_plain_parameter_reference(word, word.span),
    )
}

pub(crate) fn span_expands_only_static_pattern_safe_literals(
    checker: &Checker<'_>,
    span: Span,
    is_plain_parameter_reference: bool,
) -> bool {
    if checker.shell() != ShellDialect::Zsh {
        return false;
    }
    if !is_plain_parameter_reference {
        return false;
    }

    let mut references = checker
        .semantic()
        .references_in_span(span)
        .filter(|reference| reference_kind_can_name_pattern_operand_value(reference.kind));
    let Some(reference) = references.next() else {
        return false;
    };
    if references.next().is_some() {
        return false;
    }

    reference_expands_only_static_pattern_safe_literals(checker, reference)
}

fn reference_kind_can_name_pattern_operand_value(kind: ReferenceKind) -> bool {
    matches!(
        kind,
        ReferenceKind::Expansion
            | ReferenceKind::ParameterExpansion
            | ReferenceKind::ArrayAccess
            | ReferenceKind::ParameterPattern
            | ReferenceKind::ConditionalOperand
    )
}

fn reference_expands_only_static_pattern_safe_literals(
    checker: &Checker<'_>,
    reference: &Reference,
) -> bool {
    let bindings = checker
        .semantic_analysis()
        .reaching_bindings_for_name(&reference.name, reference.span);
    !bindings.is_empty()
        && bindings
            .iter()
            .all(|binding_id| binding_expands_to_static_pattern_safe_literal(checker, *binding_id))
}

fn binding_expands_to_static_pattern_safe_literal(
    checker: &Checker<'_>,
    binding_id: BindingId,
) -> bool {
    checker
        .facts()
        .binding_value(binding_id)
        .and_then(|value| value.scalar_word())
        .and_then(|word| static_word_text(word, checker.source()))
        .is_some_and(|text| static_literal_is_pattern_safe(&text))
}

fn static_literal_is_pattern_safe(text: &str) -> bool {
    text.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b' ' | b'_' | b'-' | b'.' | b'/' | b':' | b',' | b'=' | b'@'
            )
    })
}
