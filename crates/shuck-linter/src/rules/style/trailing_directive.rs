use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Locator, Rule, Violation};

pub struct TrailingDirective;

impl Violation for TrailingDirective {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::TrailingDirective
    }

    fn message(&self) -> String {
        "directive after code is ignored".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("move the directive before the command".to_owned())
    }
}

pub fn trailing_directive(checker: &mut Checker) {
    let locator = checker.locator();
    let spans = checker
        .facts()
        .source_facts()
        .trailing_directive_comment_spans()
        .to_vec();
    for span in spans {
        let mut diagnostic = Diagnostic::new(TrailingDirective, span);
        if let Some(fix) = trailing_directive_fix(locator, span) {
            diagnostic = diagnostic.with_fix(fix);
        }
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn trailing_directive_fix(locator: Locator<'_>, span: shuck_ast::Span) -> Option<Fix> {
    let source = locator.source();
    let line_range = locator.line_range(span.start.line)?;
    let line_start = usize::from(line_range.start());
    let raw_line_end = usize::from(line_range.end());
    let line_end = source
        .get(..raw_line_end)
        .filter(|_| raw_line_end > 0 && source.as_bytes()[raw_line_end - 1] == b'\r')
        .map_or(raw_line_end, |_| raw_line_end - 1);
    let comment_start = span.start.offset;
    if comment_start < line_start || comment_start >= line_end {
        return None;
    }

    let line = source.get(line_start..line_end)?;
    let indent_len = line
        .bytes()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    let indent = &line[..indent_len];
    let comment_text = source.get(comment_start..line_end)?;
    let newline = if raw_line_end > line_end {
        "\r\n"
    } else {
        "\n"
    };

    let mut delete_start = comment_start;
    while delete_start > line_start
        && matches!(source.as_bytes().get(delete_start - 1), Some(b' ' | b'\t'))
    {
        delete_start -= 1;
    }

    Some(Fix::safe_edits([
        Edit::insertion(line_start, format!("{indent}{comment_text}{newline}")),
        Edit::deletion_at(delete_start, line_end),
    ]))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

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
    fn reports_inline_shuck_disable_directive() {
        let source = "#!/bin/sh\n: # shuck: disable=C003\nfoo=1\n";
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
    fn ignores_shuck_disable_after_semicolon_separator() {
        let source = "#!/bin/sh\ntrue; # shuck: disable=C001\nfalse\n";
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
            "#!/bin/sh\nif true; then { # shellcheck disable=SC2086\n  echo $foo\n}; fi\n",
            "#!/bin/sh\nif true; then ( # shellcheck disable=SC2086\n  echo $foo\n); fi\n",
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
    fn ignores_directive_after_zsh_brace_control_flow_headers() {
        let sources = [
            "#!/bin/zsh\nif [[ -n $foo ]] { # shellcheck disable=SC2086\n  echo $foo\n}\n",
            "#!/bin/zsh\nif [[ -n $foo ]] { :\n} elif [[ -n $bar ]] { # shellcheck disable=SC2086\n  echo $foo\n}\n",
            "#!/bin/zsh\nif [[ -n $foo ]] { :\n} else { # shellcheck disable=SC2086\n  echo $foo\n}\n",
            "#!/bin/zsh\nfor item in 1; { # shellcheck disable=SC2086\n  echo $foo\n}\n",
            "#!/bin/zsh\nrepeat 2 { # shellcheck disable=SC2086\n  echo $foo\n}\n",
            "#!/bin/zsh\nforeach item (1 2) { # shellcheck disable=SC2086\n  echo $foo\n}\n",
        ];

        for source in sources {
            let diagnostics = test_snippet(
                source,
                &LinterSettings::for_rule(Rule::TrailingDirective).with_shell(ShellDialect::Zsh),
            );
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

    #[test]
    fn reports_keyword_like_arguments_with_trailing_shuck_disable() {
        let source = "#!/bin/sh\necho if # shuck: disable=S001\necho $foo\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn reports_keyword_suffixes_inside_words_with_trailing_directives() {
        let source =
            "#!/bin/sh\nfor item in to-do # shellcheck disable=SC2086\ndo\n  echo $foo\ndone\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::TrailingDirective));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }

    #[test]
    fn applies_safe_fix_to_move_directive_before_command() {
        let source = "#!/bin/sh\n  : # shellcheck disable=2034\nfoo=1\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::TrailingDirective),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\n  # shellcheck disable=2034\n  :\nfoo=1\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
