use crate::{Checker, ExpansionContext, Rule, Violation};

pub struct TemplateBraceInCommand;

impl Violation for TemplateBraceInCommand {
    fn rule() -> Rule {
        Rule::TemplateBraceInCommand
    }

    fn message(&self) -> String {
        "template placeholder `{{...}}` appears where a command name is expected".to_owned()
    }
}

pub fn template_brace_in_command(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .expansion_word_facts(ExpansionContext::CommandName)
        .map(|fact| fact.span())
        .filter(|span| contains_template_placeholder(span.slice(source)))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || TemplateBraceInCommand);
}

fn contains_template_placeholder(text: &str) -> bool {
    let Some(start) = text.find("{{") else {
        return false;
    };
    text[start + 2..].contains("}}")
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_double_brace_placeholders_in_command_position() {
        let source = "\
#!/bin/bash
\"$root/pkg/{{name}}/bin/{{cmd}}\" \"$@\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TemplateBraceInCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn ignores_placeholders_outside_command_position_or_without_balanced_markers() {
        let source = "\
#!/bin/bash
echo \"{{name}}\"
command \"{{tool}}\"
printf '%s\\n' \"$root/{{name}}/bin/{{cmd}}\"
echo hi > \"{{name}}\"
\"$root/bin/{{\"
\"$root/bin/}}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TemplateBraceInCommand),
        );

        assert!(diagnostics.is_empty());
    }
}
