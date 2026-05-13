use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct IfDollarCommand;

impl Violation for IfDollarCommand {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::IfDollarCommand
    }

    fn message(&self) -> String {
        "use the command's exit status directly instead of executing the output from `$(...)`"
            .to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("remove the command substitution wrapper".to_owned())
    }
}

pub fn if_dollar_command(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .command_substitution_command_spans()
        .iter()
        .copied()
        .filter_map(|span| if_dollar_command_fix(span, source))
        .map(|(span, fix)| Diagnostic::new(IfDollarCommand, span).with_fix(fix))
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn if_dollar_command_fix(span: Span, source: &str) -> Option<(Span, Fix)> {
    let text = span.slice(source);
    let body = text.strip_prefix("$(")?.strip_suffix(')')?;
    let close_cleanup = command_substitution_close_cleanup(span, source);
    Some((
        span,
        Fix::unsafe_edit(Edit::replacement_at(
            span.start.offset,
            close_cleanup.end,
            close_cleanup.replacement_body(body),
        )),
    ))
}

struct CloseCleanup<'a> {
    end: usize,
    list_operator: Option<&'a str>,
}

impl CloseCleanup<'_> {
    fn replacement_body(&self, body: &str) -> String {
        match self.list_operator {
            Some(operator) => body_with_trailing_list_operator(body, operator),
            None => body.to_owned(),
        }
    }
}

fn command_substitution_close_cleanup<'a>(span: Span, source: &'a str) -> CloseCleanup<'a> {
    let close_start = span.end.offset.saturating_sub(1);
    if !offset_is_indented_line_start(source, close_start) {
        return CloseCleanup {
            end: span.end.offset,
            list_operator: None,
        };
    }

    let mut end = span.end.offset;
    while source
        .as_bytes()
        .get(end)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        end += 1;
    }
    let Some((operator, operator_len)) = close_cleanup_operator(source, end) else {
        return CloseCleanup {
            end: span.end.offset,
            list_operator: None,
        };
    };

    end += operator_len;
    while source
        .as_bytes()
        .get(end)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        end += 1;
    }

    CloseCleanup {
        end,
        list_operator: (operator != ";").then_some(operator),
    }
}

fn close_cleanup_operator(source: &str, offset: usize) -> Option<(&str, usize)> {
    let rest = source.get(offset..)?;
    if rest.starts_with(';') {
        Some((";", 1))
    } else if rest.starts_with("&&") {
        Some(("&&", 2))
    } else if rest.starts_with("||") {
        Some(("||", 2))
    } else {
        None
    }
}

fn body_with_trailing_list_operator(body: &str, operator: &str) -> String {
    let trimmed = body.trim_end_matches([' ', '\t']);
    if let Some(without_newline) = trimmed.strip_suffix('\n') {
        format!("{without_newline} {operator} ")
    } else {
        format!("{trimmed} {operator} ")
    }
}

fn offset_is_indented_line_start(source: &str, offset: usize) -> bool {
    let line_start = source[..offset].rfind('\n').map_or(0, |offset| offset + 1);
    source[line_start..offset]
        .bytes()
        .all(|byte| matches!(byte, b' ' | b'\t'))
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn reports_command_substitution_condition_commands() {
        let source = "\
#!/bin/bash
$(false) && echo x
! $(false)
$(false)
if $(python3 -c 'import sys' 2>/dev/null); then echo ok; fi
while $(false); do break; done
until $(false); do break; done
if ! $(false); then echo no; fi
if foo && $(false); then :; fi
if $(false); echo ok; then :; fi
if $(false) | cat; then :; fi
if cat | $(false); then :; fi
if { $(false); }; then :; fi
if ( $(false) ); then :; fi
if time $(false); then :; fi
echo \"$(if $(false); then :; fi)\"
cat <(if $(false); then :; fi)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IfDollarCommand));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$(false)",
                "$(false)",
                "$(false)",
                "$(python3 -c 'import sys' 2>/dev/null)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
                "$(false)",
            ]
        );
    }

    #[test]
    fn ignores_non_condition_and_wrapper_argument_substitutions() {
        let source = "\
#!/bin/bash
$(false) --arg
if foo; then :; fi
if \"$(printf '%s' foo)\"; then :; fi
if [[ \"$pm\" == apt ]] && \"$(printf '%s' missing)\" != installed; then :; fi
if $(command -v rvm) -v > /dev/null 2>&1; then :; fi
if $(tc-getSTRIP) --enable-deterministic-archives |& grep -q aarch; then :; fi
if command $(false); then :; fi
if env FOO=1 $(false); then :; fi
if `printf '%s' ok`; then :; fi
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::IfDollarCommand));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_by_unwrapping_command_substitution_condition() {
        let source = "#!/bin/bash\nif $(false); then echo ok; fi\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::IfDollarCommand),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/bash\nif false; then echo ok; fi\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_multiline_command_substitution_condition() {
        let source = "\
#!/bin/bash
if $(
  false
); then
  :
fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::IfDollarCommand),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            concat!(
                "#!/bin/bash\n",
                "if \n",
                "  false\n",
                "then\n",
                "  :\n",
                "fi\n",
            )
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_multiline_command_substitution_condition_with_list_operator() {
        let source = "\
#!/bin/bash
if $(
  false
)&& echo x
then
  :
fi
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::IfDollarCommand),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            concat!(
                "#!/bin/bash\n",
                "if \n",
                "  false && echo x\n",
                "then\n",
                "  :\n",
                "fi\n",
            )
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
