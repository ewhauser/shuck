use crate::{Checker, Rule, Violation};

pub struct BareSlashMarker;

impl Violation for BareSlashMarker {
    fn rule() -> Rule {
        Rule::BareSlashMarker
    }

    fn message(&self) -> String {
        "a lone `*/` token is not valid shell syntax".to_owned()
    }
}

pub fn bare_slash_marker(checker: &mut Checker) {
    let source = checker.source();
    let spans = checker
        .facts()
        .structural_commands()
        .filter(|fact| fact.wrappers().is_empty())
        .filter(|fact| fact.arena_body_args(source).is_empty())
        .filter_map(|fact| fact.arena_body_name_word(source).map(|word| word.span()))
        .filter(|span| span.slice(source) == "*/")
        .collect::<Vec<_>>();

    checker.report_all(spans, || BareSlashMarker);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_lone_slash_star_tokens_used_as_commands() {
        let source = "#!/bin/sh\n*/\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BareSlashMarker));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "*/");
    }

    #[test]
    fn ignores_slash_star_in_non_command_positions() {
        let source = "\
#!/bin/sh
echo */
printf '%s\\n' \"*/\"
printf '%s\\n' \\*/
*/ echo hi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BareSlashMarker));

        assert!(diagnostics.is_empty());
    }
}
