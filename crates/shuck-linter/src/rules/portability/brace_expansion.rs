use crate::{Checker, ExpansionContext, Rule, ShellDialect, Violation};

pub struct BraceExpansion;

impl Violation for BraceExpansion {
    fn rule() -> Rule {
        Rule::BraceExpansion
    }

    fn message(&self) -> String {
        "brace expansion is not portable in `sh`".to_owned()
    }
}

pub fn brace_expansion(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let spans = checker
        .facts()
        .word_facts()
        .iter()
        .filter(|fact| {
            !matches!(
                fact.expansion_context(),
                Some(
                    ExpansionContext::CasePattern
                        | ExpansionContext::ConditionalPattern
                        | ExpansionContext::ParameterPattern
                )
            )
        })
        .flat_map(|fact| fact.word().brace_syntax().iter().copied())
        .filter(|brace| brace.expands())
        .map(|brace| brace.span)
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || BraceExpansion);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_each_active_brace_expansion() {
        let source = "\
#!/bin/sh
echo prefix{a,b}suffix file{1..3}.txt
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BraceExpansion));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["{a,b}", "{1..3}"]
        );
    }

    #[test]
    fn ignores_quoted_and_pattern_only_brace_syntax() {
        let source = "\
#!/bin/sh
printf '%s\n' \"{a,b}\" '{1..3}'
case \"$value\" in
    {a,b}) printf '%s\n' ok ;;
esac
echo \"${name/{a,b}/x}\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BraceExpansion));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_brace_expansion_in_bash() {
        let source = "echo {a,b}\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::BraceExpansion).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }
}
