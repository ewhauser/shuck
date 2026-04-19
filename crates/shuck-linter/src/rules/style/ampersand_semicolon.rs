use crate::{Checker, Rule, Violation};

pub struct AmpersandSemicolon;

impl Violation for AmpersandSemicolon {
    fn rule() -> Rule {
        Rule::AmpersandSemicolon
    }

    fn message(&self) -> String {
        "background command should not be followed by `;`".to_owned()
    }
}

pub fn ampersand_semicolon(checker: &mut Checker) {
    checker.report_all_dedup(
        checker.facts().background_semicolon_spans().to_vec(),
        || AmpersandSemicolon,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_background_followed_by_semicolon() {
        let source = "#!/bin/sh\necho x &;\necho y & ;\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AmpersandSemicolon));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), ";");
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.slice(source), ";");
        assert_eq!(diagnostics[1].span.start.line, 3);
    }

    #[test]
    fn ignores_background_without_semicolon() {
        let source = "#!/bin/sh\necho x &\nwait\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AmpersandSemicolon));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_case_item_terminators_after_background() {
        let source = "\
#!/bin/bash
case ${1-} in
  break) printf '%s\\n' ok &;;
  spaced) printf '%s\\n' ok & ;;
  fallthrough) printf '%s\\n' ok & ;&
  continue) printf '%s\\n' ok & ;;&
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::AmpersandSemicolon));

        assert!(diagnostics.is_empty());
    }
}
