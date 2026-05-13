use shuck_ast::Span;

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct SshLocalExpansion;

impl Violation for SshLocalExpansion {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::SshLocalExpansion
    }

    fn message(&self) -> String {
        "ssh command text is expanded locally before the remote shell sees it".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("single-quote the remote command".to_owned())
    }
}

pub fn ssh_local_expansion(checker: &mut Checker) {
    let source = checker.source();
    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| fact.options().ssh())
        .flat_map(|fact| {
            fact.local_expansion_spans().iter().copied().map(|span| {
                Diagnostic::new(SshLocalExpansion, span)
                    .with_fix(remote_command_fix(source, fact.remote_command_arg_span()))
            })
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn remote_command_fix(source: &str, span: Span) -> Fix {
    let replacement = single_quoted_remote_command(span.slice(source)).unwrap_or_else(|| {
        format!(
            "'{}'",
            span.slice(source).trim_matches('"').replace('\'', "'\\''")
        )
    });
    Fix::unsafe_edit(Edit::replacement(replacement, span))
}

fn single_quoted_remote_command(text: &str) -> Option<String> {
    let body = text.strip_prefix('"')?.strip_suffix('"')?;
    let body = decode_double_quoted_body(body);
    Some(format!("'{}'", body.replace('\'', "'\\''")))
}

fn decode_double_quoted_body(body: &str) -> String {
    let mut decoded = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }

        match chars.next() {
            Some(ch @ ('$' | '`' | '"' | '\\')) => decoded.push(ch),
            Some('\n') => {}
            Some(ch) => {
                decoded.push('\\');
                decoded.push(ch);
            }
            None => decoded.push('\\'),
        }
    }
    decoded
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule};

    #[test]
    fn ignores_expansions_in_destination_arguments() {
        let source = "\
#!/bin/sh
ssh
ssh \"$host\"
ssh \"$host\" printf '%s\\n' ok
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_expansions_in_remote_command_arguments() {
        let source = "\
#!/bin/sh
ssh \"$host\" \"echo $HOME\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].span.slice(source), "$HOME");
    }

    #[test]
    fn ignores_local_ssh_options_before_destination() {
        let source = "\
#!/bin/sh
ssh -i \"$key\" \"$host\" \"echo $HOME\"
ssh -o BatchMode=yes host \"echo $USER\"
ssh -- host \"echo $PATH\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_non_terminal_and_assignment_style_remote_expansions() {
        let source = "\
#!/bin/sh
ssh \"$host\" cmd \"$HOME\" --force
ssh \"$host\" HELLO=\"$HOME\"
ssh \"$host\" foo=\"$USER\" bar
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_only_the_final_fully_quoted_remote_argument() {
        let source = "\
#!/bin/sh
ssh \"$host\" \"$HOME\" \"$USER\"
ssh \"$host\" cmd \"$HOME\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].span.slice(source), "$USER");
        assert_eq!(diagnostics[1].span.slice(source), "$HOME");
    }

    #[test]
    fn ignores_remote_command_shapes_with_leading_dash_arguments() {
        let source = "\
#!/bin/sh
ssh host -t \"echo $HOME\"
ssh host ls -l \"$HOME\"
ssh host cmd --flag \"$HOME\"
ssh host cmd \"--flag\" \"$HOME\"
ssh host cmd '-t' \"$USER\"
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_expansions_inside_command_substitutions() {
        let source = "\
#!/bin/sh
URL=$(ssh \"$host\" url \"$REPO\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SshLocalExpansion));

        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].span.slice(source), "$REPO");
    }

    #[test]
    fn applies_unsafe_fix_to_single_quote_remote_command_argument() {
        let source = "#!/bin/sh\nssh \"$host\" \"echo $HOME\"\nssh host \"printf '%s\\n' $USER\"\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SshLocalExpansion),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nssh \"$host\" 'echo $HOME'\nssh host 'printf '\\''%s\\n'\\'' $USER'\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn decodes_double_quote_escapes_when_fixing_remote_command() {
        let source = "#!/bin/sh\nssh host \"printf \\\"%s\\\" $USER \\$HOME \\\\tmp\"\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::SshLocalExpansion),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nssh host 'printf \"%s\" $USER $HOME \\tmp'\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
