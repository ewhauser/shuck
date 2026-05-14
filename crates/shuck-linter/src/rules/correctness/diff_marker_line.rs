use shuck_ast::Span;
use shuck_semantic::{CommandConditionRole, CommandId};

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct DiffMarkerLine;

impl Violation for DiffMarkerLine {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::DiffMarkerLine
    }

    fn message(&self) -> String {
        "this dash-prefixed command looks like leftover patch text".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("comment out the dash-prefixed line".to_owned())
    }
}

pub fn diff_marker_line(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let source = checker.source();
    let command_facts = checker.facts().command_facts();
    let mut diagnostics = command_facts
        .structural_commands()
        .filter(|command| command.command_name_starts_with_literal_dash(source))
        .filter(|command| !command.command_name_follows_escaped_semicolon(source))
        .filter_map(|command| {
            let name = command.command_name_word()?;
            let can_fix_in_context = command_can_be_fixed_in_context(checker, command.id());
            Some(diagnostic_for_dash_command(
                source,
                name.span,
                can_fix_in_context,
            ))
        })
        .collect::<Vec<_>>();
    diagnostics.extend(
        checker
            .facts()
            .source_facts()
            .escaped_dash_command_name_spans()
            .iter()
            .copied()
            .map(|span| {
                let can_fix_in_context = command_facts
                    .command_for_name_word_span(span)
                    .is_none_or(|command| command_can_be_fixed_in_context(checker, command.id()));
                diagnostic_for_dash_command(source, span, can_fix_in_context)
            }),
    );

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn command_can_be_fixed_in_context(checker: &Checker<'_>, command_id: CommandId) -> bool {
    !matches!(
        checker.semantic().command_condition_role(command_id),
        Some(
            CommandConditionRole::If
                | CommandConditionRole::Elif
                | CommandConditionRole::While
                | CommandConditionRole::Until
        )
    ) && !command_satisfies_pending_operator(checker, command_id)
}

fn command_satisfies_pending_operator(checker: &Checker<'_>, command_id: CommandId) -> bool {
    let command_facts = checker.facts().command_facts();
    command_facts.pipelines().iter().any(|pipeline| {
        pipeline
            .segments()
            .iter()
            .skip(1)
            .any(|segment| segment.command_id() == command_id)
    }) || command_facts.lists().iter().any(|list| {
        list.segments()
            .iter()
            .skip(1)
            .any(|segment| segment.command_id() == command_id)
    })
}

fn diagnostic_for_dash_command(source: &str, span: Span, can_fix_in_context: bool) -> Diagnostic {
    let diagnostic = Diagnostic::new(DiffMarkerLine, span);
    if can_fix_in_context && span_starts_physical_line(source, span) {
        diagnostic.with_fix(Fix::unsafe_edit(Edit::insertion(span.start.offset, "# ")))
    } else {
        diagnostic
    }
}

fn span_starts_physical_line(source: &str, span: Span) -> bool {
    let line_start = source[..span.start.offset]
        .rfind('\n')
        .map_or(0, |offset| offset + 1);
    source[line_start..span.start.offset]
        .chars()
        .all(|ch| matches!(ch, ' ' | '\t'))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_dash_prefixed_command_names() {
        let source = "\
#!/bin/sh
--- a/sample.txt
  --- indented.txt
--help
\\-n foo
\\-x&& echo ok
-$tool foo
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DiffMarkerLine));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["---", "---", "--help", "\\-n", "\\-x", "-$tool"]
        );
    }

    #[test]
    fn ignores_arguments_comments_quoted_names_and_zsh() {
        let source = "\
#!/bin/sh
echo --- a/sample.txt
# --- comment.txt
cat <<'DOC'
--- heredoc.txt
DOC
\"-n\" foo
'-n' foo
command -n foo
find . -exec chmod 755 {}\\; -o -name '*.txt'
echo \\
\\-n
case \"$1\" in
  \\-n) echo no ;;
esac
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DiffMarkerLine));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");

        let zsh_diagnostics = test_snippet(
            "#!/bin/zsh\n--- a/sample.txt\n",
            &LinterSettings::for_rule(Rule::DiffMarkerLine).with_shell(ShellDialect::Zsh),
        );
        assert!(
            zsh_diagnostics.is_empty(),
            "diagnostics: {zsh_diagnostics:?}"
        );
    }

    #[test]
    fn applies_unsafe_fix_before_the_command_name() {
        let source = "#!/bin/sh\n  --- a/sample.txt\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DiffMarkerLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(result.fixed_source, "#!/bin/sh\n  # --- a/sample.txt\n");
    }

    #[test]
    fn withholds_fix_for_dash_commands_inside_control_heads() {
        let source = "#!/bin/sh\nif --help; then echo ok; fi\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DiffMarkerLine));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "--help");
        assert!(diagnostics[0].fix.is_none());

        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DiffMarkerLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
    }

    #[test]
    fn withholds_fix_for_dash_commands_inside_multiline_control_heads() {
        let source = "\
#!/bin/sh
if
--help
then
  echo ok
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DiffMarkerLine));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "--help");
        assert!(diagnostics[0].fix.is_none());

        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DiffMarkerLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
    }

    #[test]
    fn withholds_fix_for_escaped_dash_commands_inside_multiline_control_heads() {
        let source = "\
#!/bin/sh
if
\\-n;
then
  echo ok
fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DiffMarkerLine));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "\\-n");
        assert!(diagnostics[0].fix.is_none());

        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DiffMarkerLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
    }

    #[test]
    fn withholds_fix_for_dash_commands_inside_loop_condition_heads() {
        let source = "\
#!/bin/sh
while
--help
do
  echo ok
done
until
\\-n
do
  echo ok
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DiffMarkerLine));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["--help", "\\-n"]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );

        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DiffMarkerLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
    }

    #[test]
    fn withholds_fix_for_dash_commands_satisfying_pending_operators() {
        let source = "\
#!/bin/sh
foo |
--help
foo &&
\\-n
foo ||
--- retry
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::DiffMarkerLine));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["--help", "\\-n", "---"]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.fix.is_none())
        );

        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::DiffMarkerLine),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
    }
}
