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
    {
        return false;
    }

    true
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
