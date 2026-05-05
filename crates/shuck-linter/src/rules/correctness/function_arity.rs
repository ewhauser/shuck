use shuck_semantic::ScopeId;

use crate::facts::conditionals::{
    ConditionalNodeFact, ConditionalOperandFact, ConditionalOperatorFamily,
};
use crate::{Checker, ShellDialect};

pub(crate) fn zsh_function_arity_is_externally_defined(
    checker: &Checker<'_>,
    function_scope: ScopeId,
) -> bool {
    checker.shell() == ShellDialect::Zsh
        && (checker
            .facts()
            .function_is_external_entrypoint(function_scope)
            || checker
                .facts()
                .function_positional_parameter_facts(function_scope)
                .required_arg_count()
                == 0
            || zsh_function_has_optional_first_positional_parameter(checker, function_scope))
}

fn zsh_function_has_optional_first_positional_parameter(
    checker: &Checker<'_>,
    function_scope: ScopeId,
) -> bool {
    let positional = checker
        .facts()
        .function_positional_parameter_facts(function_scope);
    if positional.required_arg_count() != 1 {
        return false;
    }

    checker.facts().commands().iter().any(|command| {
        checker.semantic().enclosing_function_scope(command.scope()) == Some(function_scope)
            && command
                .conditional()
                .is_some_and(conditional_mentions_optional_first_positional_parameter)
    })
}

fn conditional_mentions_optional_first_positional_parameter(
    conditional: &crate::facts::conditionals::ConditionalFact<'_>,
) -> bool {
    conditional.nodes().iter().any(|node| match node {
        ConditionalNodeFact::BareWord(fact) => {
            operand_is_optional_first_positional_parameter(fact.operand())
        }
        ConditionalNodeFact::Unary(fact) if fact.is_empty_string_test() => {
            operand_is_optional_first_positional_parameter(fact.operand())
        }
        ConditionalNodeFact::Binary(fact)
            if fact.operator_family() == ConditionalOperatorFamily::StringBinary =>
        {
            operand_is_optional_first_positional_parameter(fact.left())
                || operand_is_optional_first_positional_parameter(fact.right())
        }
        ConditionalNodeFact::Unary(_)
        | ConditionalNodeFact::Binary(_)
        | ConditionalNodeFact::Other(_) => false,
    })
}

fn operand_is_optional_first_positional_parameter(operand: ConditionalOperandFact<'_>) -> bool {
    operand.is_optional_first_positional_parameter()
}
