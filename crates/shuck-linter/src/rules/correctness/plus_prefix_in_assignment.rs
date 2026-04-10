use shuck_ast::{Command, DeclOperand, Span, Word};

use crate::{Checker, Rule, Violation};

pub struct PlusPrefixInAssignment;

impl Violation for PlusPrefixInAssignment {
    fn rule() -> Rule {
        Rule::PlusPrefixInAssignment
    }

    fn message(&self) -> String {
        "leading `+` makes this look like a command instead of an assignment".to_owned()
    }
}

pub fn plus_prefix_in_assignment(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .flat_map(|fact| match fact.command() {
            Command::Simple(command) => assignment_like_plus_span(&command.name, source)
                .into_iter()
                .collect::<Vec<_>>(),
            Command::Decl(command) => command
                .operands
                .iter()
                .filter_map(|operand| match operand {
                    DeclOperand::Dynamic(word) => assignment_like_plus_span(word, source),
                    DeclOperand::Flag(_) | DeclOperand::Name(_) | DeclOperand::Assignment(_) => {
                        None
                    }
                })
                .collect::<Vec<_>>(),
            Command::Builtin(_)
            | Command::Binary(_)
            | Command::Compound(_)
            | Command::Function(_)
            | Command::AnonymousFunction(_) => Vec::new(),
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || PlusPrefixInAssignment);
}

fn assignment_like_plus_span(word: &Word, source: &str) -> Option<Span> {
    let text = word.span.slice(source);
    let remainder = text.strip_prefix('+')?;
    let target_end = remainder.find("+=").or_else(|| remainder.find('='))?;
    let target = &remainder[..target_end];

    is_valid_identifier(target).then_some(word.span)
}

fn is_valid_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some('A'..='Z' | 'a'..='z' | '_'))
        && chars.all(|character| matches!(character, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_'))
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_assignment_like_words_with_a_leading_plus() {
        let source = "\
#!/bin/bash
+YYYY=\"$( date +%Y )\"
export +MONTH=12
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PlusPrefixInAssignment),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["+YYYY=\"$( date +%Y )\"", "+MONTH=12"]
        );
    }

    #[test]
    fn ignores_regular_commands_and_non_identifier_targets() {
        let source = "\
#!/bin/sh
echo +YEAR=2024
+1=bad
name+=still_ok
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PlusPrefixInAssignment),
        );

        assert!(diagnostics.is_empty());
    }
}
