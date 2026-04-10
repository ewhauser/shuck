use crate::{Checker, Rule, Violation};

pub struct LiteralBraces;

impl Violation for LiteralBraces {
    fn rule() -> Rule {
        Rule::LiteralBraces
    }

    fn message(&self) -> String {
        "literal braces may be interpreted as brace syntax".to_owned()
    }
}

pub fn literal_braces(checker: &mut Checker) {
    checker.report_all_dedup(checker.facts().literal_brace_spans().to_vec(), || {
        LiteralBraces
    });
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_literal_unquoted_brace_pair_edges() {
        let source = "#!/bin/bash\necho HEAD@{1}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.column, 11);
        assert_eq!(diagnostics[1].span.start.column, 13);
    }

    #[test]
    fn ignores_quoted_and_expanding_braces() {
        let source = "#!/bin/bash\necho \"HEAD@{1}\" x{a,b}y\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_find_exec_placeholder_and_regex_quantifier() {
        let source = "\
#!/bin/bash
find . -exec echo {} \\;
if [[ \"$hash\" =~ ^[a-f0-9]{40}$ ]]; then
  :
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_literal_braces_for_non_find_exec_forms() {
        let source = "\
#!/bin/bash
echo {} +
myfind -exec echo {} \\;
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 4);
    }

    #[test]
    fn reports_escaped_dollar_literal_braces() {
        let source = "\
#!/bin/bash
eval command sudo \\\"\\${sudo_args[@]}\\\"
echo [0-9a-f]{$HASHLEN}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LiteralBraces));

        assert_eq!(diagnostics.len(), 4);
    }
}
