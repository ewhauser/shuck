use crate::{Checker, Rule, Violation};

pub struct TrailingDirective;

impl Violation for TrailingDirective {
    fn rule() -> Rule {
        Rule::TrailingDirective
    }

    fn message(&self) -> String {
        "directive after code is ignored".to_owned()
    }
}

pub fn trailing_directive(checker: &mut Checker) {
    checker.report_all_dedup(
        checker.facts().trailing_directive_comment_spans().to_vec(),
        || TrailingDirective,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_inline_disable_directive() {
        let source = "#!/bin/sh\n: # shellcheck disable=2034\nfoo=1\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[0].span.start.column, 3);
        assert_eq!(diagnostics[0].span.slice(source), "#");
    }

    #[test]
    fn ignores_own_line_directive() {
        let source = "#!/bin/sh\n# shellcheck disable=2034\nfoo=1\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_non_directive_inline_comment() {
        let source = "#!/bin/sh\n: # shellcheck reminder\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_directive_after_subshell_opener() {
        let source = "#!/bin/sh\n( # shellcheck disable=SC2164\n  cd /tmp\n)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_directive_after_brace_group_opener() {
        let source = "#!/bin/sh\n{ # shellcheck disable=SC2164\n  cd /tmp\n}\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_directive_after_semicolon_separator() {
        let source = "#!/bin/sh\ntrue; # shellcheck disable=SC2317\nfalse\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_directive_after_case_label() {
        let source =
            "#!/bin/sh\ncase $x in\n  on) # shellcheck disable=SC2034\n    :\n    ;;\nesac\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_directive_after_control_flow_headers() {
        let sources = [
            "#!/bin/sh\nif # shellcheck disable=SC2086\n  echo $foo\nthen\n  :\nfi\n",
            "#!/bin/sh\nif true; then # shellcheck disable=SC2086\n  echo $foo\nfi\n",
            "#!/bin/sh\nif false; then\n  :\nelif true; then # shellcheck disable=SC2086\n  echo $foo\nfi\n",
            "#!/bin/sh\nif false; then\n  :\nelse # shellcheck disable=SC2086\n  echo $foo\nfi\n",
            "#!/bin/sh\nfor item in 1; do # shellcheck disable=SC2086\n  echo $foo\ndone\n",
            "#!/bin/sh\nwhile # shellcheck disable=SC2086\n  echo $foo\ndo\n  :\ndone\n",
            "#!/bin/sh\nuntil # shellcheck disable=SC2086\n  echo $foo\ndo\n  :\ndone\n",
        ];

        for source in sources {
            let diagnostics =
                test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));
            assert!(diagnostics.is_empty(), "{source}");
        }
    }

    #[test]
    fn reports_keyword_like_arguments_with_trailing_directives() {
        let source = "#!/bin/sh\necho if # shellcheck disable=SC2086\necho $foo\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }
}
