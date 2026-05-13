use crate::facts::word_spans;
use shuck_ast::Span;

use crate::{
    Checker, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation,
    WordFactHostKind,
};

pub struct SuspiciousBracketGlob;

impl Violation for SuspiciousBracketGlob {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SuspiciousBracketGlob
    }

    fn message(&self) -> String {
        "bracket globs only match one character at a time; quote word-like ones".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("quote the bracket-glob text".to_owned())
    }
}

pub fn suspicious_bracket_glob(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| {
            fact.body_name_word()
                .is_none_or(|word| !bare_bracket_test_name(word.span.slice(source)))
        })
        .flat_map(|fact| fact.suspicious_body_name_bracket_glob_spans(source))
        .chain(checker.facts().case_items().iter().flat_map(|item| {
            word_spans::case_item_suspicious_bracket_glob_spans(item.item(), source)
        }))
        .chain(
            checker
                .facts()
                .commands()
                .iter()
                .filter_map(|fact| fact.conditional())
                .flat_map(|conditional| {
                    word_spans::conditional_suspicious_bracket_glob_spans(
                        conditional.expression(),
                        source,
                    )
                }),
        )
        .chain(
            checker
                .facts()
                .word_facts()
                .iter()
                .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
                .filter(|fact| supports_suspicious_bracket_glob_context(fact.expansion_context()))
                .flat_map(|fact| fact.suspicious_bracket_glob_spans(source)),
        )
        .map(|span| {
            Diagnostic::new(SuspiciousBracketGlob, span)
                .with_fix(Fix::unsafe_edit(single_quote_span_edit(span, source)))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn single_quote_span_edit(span: Span, source: &str) -> Edit {
    Edit::replacement(format!("'{}'", span.slice(source)), span)
}

fn supports_suspicious_bracket_glob_context(context: Option<ExpansionContext>) -> bool {
    matches!(
        context,
        Some(
            ExpansionContext::CommandArgument
                | ExpansionContext::AssignmentValue
                | ExpansionContext::DeclarationAssignmentValue
                | ExpansionContext::RedirectTarget(_)
                | ExpansionContext::ForList
                | ExpansionContext::SelectList
                | ExpansionContext::CasePattern
                | ExpansionContext::ConditionalPattern
                | ExpansionContext::StringTestOperand
        )
    )
}

fn bare_bracket_test_name(text: &str) -> bool {
    text == "["
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_suspicious_bracket_globs_across_shell_contexts() {
        let source = "\
#!/bin/bash
[appname] arg
foo[appname] arg
echo [skipped]
printf '%s\\n' \"$dir\"/[appname]
ITEM=[0,-1,1,-10,-20]
cat <<EOF >/etc/systemd/system/[appname].service
EOF
for target in [appname]; do :; done
case $x in [appname]) : ;; esac
[ \"$x\" = foo[appname]bar ]
[[ $x = [appname] ]]
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SuspiciousBracketGlob),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "[appname]",
                "[appname]",
                "[skipped]",
                "[appname]",
                "[0,-1,1,-10,-20]",
                "[appname]",
                "[appname]",
                "[appname]",
                "[appname]",
                "[appname]"
            ]
        );
    }

    #[test]
    fn applies_unsafe_fix_by_single_quoting_bracket_glob_text() {
        let source = "#!/bin/bash\necho [appname]\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SuspiciousBracketGlob),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/bash\necho '[appname]'\n");
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn ignores_valid_sets_literal_text_and_parameter_patterns() {
        let source = "\
echo [ab]
echo [a-z]
echo [[:alpha:]]
echo foo[bar]baz
tr [:lower:] [:upper:]
sed -r s/[^a-zA-Z0-9]+/-/g
case \"$1\" in
  [0-9a-fA-F][0-9a-fA-F]) ;;
  *[!a-zA-Z_]*) return 1 ;;
esac
echo \"[appname]\"
echo \\[appname\\]
foo=${bar#[appname]}
foo=${bar%[appname]}
if [ \"$ARCH\" = \"x86_64\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SuspiciousBracketGlob),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_zsh_brace_ccl_word_like_character_classes_from_facts() {
        let source = "\
setopt brace_ccl
{appname} arg
echo {appname}
opt=brace_ccl
setopt \"$opt\"
{dynamicc} arg
echo {dynamicc}
unsetopt brace_ccl
{appname} arg
echo {appname}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::SuspiciousBracketGlob).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["{appname}", "{appname}", "{dynamicc}", "{dynamicc}"]
        );
    }
}
