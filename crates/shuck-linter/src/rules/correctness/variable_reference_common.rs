use shuck_ast::Name;
use shuck_semantic::{BindingKind, Reference, ReferenceKind};

use crate::Checker;

#[derive(Debug, Clone, Copy)]
pub(super) struct VariableReferenceFilter {
    pub suppress_environment_style_names: bool,
}

pub(super) fn is_reportable_variable_reference(
    checker: &Checker<'_>,
    reference: &Reference,
    filter: VariableReferenceFilter,
) -> bool {
    if matches!(
        reference.kind,
        ReferenceKind::DeclarationName | ReferenceKind::ImplicitRead
    ) {
        return false;
    }
    if is_shell_special_parameter(reference.name.as_str()) {
        return false;
    }
    if filter.suppress_environment_style_names && is_environment_style_name(reference.name.as_str())
    {
        return false;
    }
    if checker
        .facts()
        .is_c006_presence_tested_name(&reference.name, reference.span)
    {
        return false;
    }
    if checker
        .facts()
        .is_suppressed_subscript_reference(reference.span)
    {
        return false;
    }
    if checker
        .semantic()
        .is_guarded_parameter_reference(reference.id)
        || checker
            .semantic()
            .is_defaulting_parameter_operand_reference(reference.id)
        || has_prior_c006_suppressing_reference(checker, reference)
    {
        return false;
    }

    true
}

fn has_prior_c006_suppressing_reference(checker: &Checker<'_>, reference: &Reference) -> bool {
    checker.semantic().references().iter().any(|candidate| {
        candidate.id != reference.id
            && candidate.name == reference.name
            && candidate.span.start.offset < reference.span.start.offset
            && c006_suppresses_later_references(checker, candidate)
    })
}

fn c006_suppresses_later_references(checker: &Checker<'_>, reference: &Reference) -> bool {
    checker
        .semantic()
        .is_guarded_parameter_reference(reference.id)
        || checker
            .semantic()
            .is_defaulting_parameter_operand_reference(reference.id)
        || suppressed_subscript_reference_suppresses_later_references(checker, reference)
}

fn suppressed_subscript_reference_suppresses_later_references(
    checker: &Checker<'_>,
    reference: &Reference,
) -> bool {
    if !checker
        .facts()
        .is_subscript_later_suppression_reference(reference.span)
    {
        return false;
    }

    checker
        .facts()
        .innermost_command_at(reference.span.start.offset)
        .and_then(|command| command.static_utility_name())
        .is_none_or(|name| !matches!(name, "unset" | "[" | "[[" | "test"))
}

pub(super) fn is_environment_style_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|char| char.is_ascii_uppercase() || char.is_ascii_digit() || char == '_')
}

pub(super) fn is_sc2154_defining_binding(kind: BindingKind) -> bool {
    !matches!(
        kind,
        BindingKind::FunctionDefinition | BindingKind::Imported
    )
}

pub(super) fn has_same_name_defining_bindings(checker: &Checker<'_>, name: &Name) -> bool {
    checker
        .semantic()
        .bindings_for(name)
        .iter()
        .copied()
        .any(|binding_id| is_sc2154_defining_binding(checker.semantic().binding(binding_id).kind))
}

fn is_shell_special_parameter(name: &str) -> bool {
    matches!(name, "@" | "*" | "#" | "?" | "-" | "$" | "!" | "0")
        || (!name.is_empty() && name.chars().all(|char| char.is_ascii_digit()))
}
