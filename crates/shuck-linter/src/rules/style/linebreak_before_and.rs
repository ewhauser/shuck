use crate::{Rule, Violation};

pub struct LinebreakBeforeAnd;

impl Violation for LinebreakBeforeAnd {
    fn rule() -> Rule {
        Rule::LinebreakBeforeAnd
    }

    fn message(&self) -> String {
        "control operator starts a new line instead of ending the previous one".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_leading_and_operator() {
        let source = "#!/bin/sh\ntrue\n&& echo x\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakBeforeAnd));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.start.column, 1);
        assert_eq!(diagnostics[0].span.slice(source), "&&");
    }

    #[test]
    fn reports_leading_or_and_pipe_operators() {
        let source = "#!/bin/sh\ntrue\n  || echo x\necho hi\n  | cat\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakBeforeAnd));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.slice(source), "||");
        assert_eq!(diagnostics[1].span.slice(source), "|");
    }

    #[test]
    fn ignores_operator_at_end_of_previous_line() {
        let source = "#!/bin/sh\ntrue &&\n  echo x\necho hi |\n  cat\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LinebreakBeforeAnd));

        assert!(diagnostics.is_empty());
    }
}
