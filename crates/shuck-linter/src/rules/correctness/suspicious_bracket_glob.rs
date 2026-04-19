use crate::{
    Checker, ExpansionContext, Rule, Violation, WordFactHostKind,
    case_item_suspicious_bracket_glob_spans, conditional_suspicious_bracket_glob_spans,
    word_suspicious_bracket_glob_spans,
};

pub struct SuspiciousBracketGlob;

impl Violation for SuspiciousBracketGlob {
    fn rule() -> Rule {
        Rule::SuspiciousBracketGlob
    }

    fn message(&self) -> String {
        "bracket globs only match one character at a time; quote word-like ones".to_owned()
    }
}

pub fn suspicious_bracket_glob(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.body_name_word())
        .filter(|word| !bare_bracket_test_name(word.span.slice(source)))
        .flat_map(|word| word_suspicious_bracket_glob_spans(word, source))
        .chain(
            checker
                .facts()
                .case_items()
                .iter()
                .flat_map(|item| case_item_suspicious_bracket_glob_spans(item.item(), source)),
        )
        .chain(
            checker
                .facts()
                .commands()
                .iter()
                .filter_map(|fact| fact.conditional())
                .flat_map(|conditional| {
                    conditional_suspicious_bracket_glob_spans(conditional.expression(), source)
                }),
        )
        .chain(
            checker
                .facts()
                .word_facts()
                .iter()
                .filter(|fact| fact.host_kind() == WordFactHostKind::Direct)
                .filter(|fact| supports_suspicious_bracket_glob_context(fact.expansion_context()))
                .flat_map(|fact| word_suspicious_bracket_glob_spans(fact.word(), source)),
        )
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || SuspiciousBracketGlob);
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
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

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
}
