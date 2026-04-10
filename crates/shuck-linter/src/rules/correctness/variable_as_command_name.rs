use shuck_semantic::ScopeKind;

use crate::{Checker, ExpansionContext, Rule, Violation, WordQuote};

pub struct VariableAsCommandName;

impl Violation for VariableAsCommandName {
    fn rule() -> Rule {
        Rule::VariableAsCommandName
    }

    fn message(&self) -> String {
        "variable expansion is being used as a command name inside a function".to_owned()
    }
}

pub fn variable_as_command_name(checker: &mut Checker) {
    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandName)
        .filter_map(|fact| {
            let command = checker.facts().command(fact.command_id());
            if !is_inside_function(checker, command) {
                return None;
            }
            if fact.classification().quote != WordQuote::Unquoted {
                return None;
            }
            if !fact.classification().has_scalar_expansion() {
                return None;
            }
            if fact.has_literal_affixes() {
                return None;
            }

            fact.scalar_expansion_spans().first().copied()
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || VariableAsCommandName);
}

fn is_inside_function(checker: &Checker<'_>, fact: &crate::facts::CommandFact<'_>) -> bool {
    let scope = checker.semantic().scope_at(fact.stmt().span.start.offset);
    checker.semantic().ancestor_scopes(scope).any(|ancestor| {
        matches!(
            checker.semantic().scope_kind(ancestor),
            ScopeKind::Function(_)
        )
    })
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_variable_command_names_inside_functions() {
        let source = "\
#!/bin/sh
f() {
  $cmd hello
}
f
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "$cmd");
    }

    #[test]
    fn ignores_quoted_variables_and_top_level_commands() {
        let source = "\
#!/bin/sh
cmd=echo
f() {
  \"$cmd\" hello
}
$cmd world
f
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_command_substitutions_without_variable_expansions() {
        let source = "\
#!/bin/sh
f() {
  $(printf echo) hello
}
f
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::VariableAsCommandName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
