use crate::{Checker, Diagnostic, Edit, ExpansionContext, Fix, FixAvailability, Rule, Violation};

pub struct LiteralBackslash;

impl Violation for LiteralBackslash {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::LiteralBackslash
    }

    fn message(&self) -> String {
        "a backslash before a normal letter is literal".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the literal backslash".to_owned())
    }
}

pub fn literal_backslash(checker: &mut Checker) {
    let source = checker.source();
    let facts = checker.facts();
    let diagnostics = checker
        .facts()
        .word_facts()
        .iter()
        .filter(|fact| is_relevant_word_context(fact.expansion_context()))
        .filter(|fact| !fact.is_nested_word_command())
        .filter(|fact| !is_command_name_word(facts, *fact))
        .filter(|fact| !is_unalias_argument(facts, *fact))
        .filter_map(|fact| fact.standalone_literal_backslash_span(source))
        .map(|span| {
            Diagnostic::new(LiteralBackslash, span).with_fix(Fix::safe_edit(Edit::deletion_at(
                span.start.offset,
                span.start.offset + 1,
            )))
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn is_relevant_word_context(context: Option<ExpansionContext>) -> bool {
    matches!(
        context,
        Some(
            ExpansionContext::CommandArgument
                | ExpansionContext::ForList
                | ExpansionContext::SelectList
        )
    )
}

fn is_command_name_word<'a>(
    facts: &'a crate::facts::LinterFacts<'a>,
    fact: crate::facts::words::WordOccurrenceRef<'_, 'a>,
) -> bool {
    facts
        .command(fact.command_id())
        .body_name_word()
        .is_some_and(|word| word.span == fact.span())
}

fn is_unalias_argument<'a>(
    facts: &'a crate::facts::LinterFacts<'a>,
    fact: crate::facts::words::WordOccurrenceRef<'_, 'a>,
) -> bool {
    fact.expansion_context() == Some(ExpansionContext::CommandArgument)
        && facts
            .command(fact.command_id())
            .effective_name_is("unalias")
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_literal_backslashes_before_normal_letters() {
        let source = "\
#!/bin/sh
echo \\q
unalias \\R
\\command \\ls -ld file
printf '%s\\n' \\q
echo \\command
echo foo\\xbar
foo=bar\\w
case x in foo\\q) : ;; esac
cat < foo\\q
echo \\n
echo \\Q
echo \"\\q\"
echo '\\q'
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBackslash));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["", ""]
        );
    }

    #[test]
    fn applies_safe_fix_to_literal_backslash_words() {
        let source = "#!/bin/sh\necho \\q\nprintf '%s\\n' \\w\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::LiteralBackslash),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(result.fixed_source, "#!/bin/sh\necho q\nprintf '%s\\n' w\n");
        assert!(result.fixed_diagnostics.is_empty());
    }
}
