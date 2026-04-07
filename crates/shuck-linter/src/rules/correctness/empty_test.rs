use crate::{Checker, Rule, SimpleTestShape, Violation};

pub struct EmptyTest;

impl Violation for EmptyTest {
    fn rule() -> Rule {
        Rule::EmptyTest
    }

    fn message(&self) -> String {
        "test expression is empty".to_owned()
    }
}

pub fn empty_test(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| {
            fact.simple_test()
                .map(|simple_test| (fact.span(), simple_test))
        })
        .filter(|(_, fact)| fact.shape() == SimpleTestShape::Empty && !fact.empty_test_suppressed())
        .map(|(span, _)| span)
        .collect::<Vec<_>>();

    for span in spans {
        checker.report(EmptyTest, span);
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule};

    #[test]
    fn shellspec_parameters_blocks_are_ignored() {
        let source = "\
Describe 'clone'
Parameters
  \"test\"
  \"test$SHELLSPEC_LF\"
End

test
";
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/ko1nksm__shellspec__spec__core__clone_spec.sh"),
            source,
            &LinterSettings::for_rule(Rule::EmptyTest),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::EmptyTest);
        assert_eq!(diagnostics[0].span.slice(source).trim_end(), "test");
    }
}
