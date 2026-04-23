use crate::{Checker, Rule, Violation};

pub struct TemplateBraceInCommand;

impl Violation for TemplateBraceInCommand {
    fn rule() -> Rule {
        Rule::TemplateBraceInCommand
    }

    fn message(&self) -> String {
        "this token is being treated as a command name, but it looks more like stray text"
            .to_owned()
    }
}

pub fn template_brace_in_command(checker: &mut Checker) {
    let source = checker.source();
    let facts = checker.facts();
    let spans = facts
        .commands()
        .iter()
        .filter(|command| command.wrappers().is_empty())
        .filter_map(|command| {
            let span = command.body_word_span()?;
            let trailing_literal_char = facts
                .any_word_fact(span)
                .and_then(|word| word.trailing_literal_char());
            let suspicious_word_shape = command.body_word_contains_template_placeholder(source)
                || command
                    .body_word_has_suspicious_quoted_command_trailer(source, trailing_literal_char)
                || command.body_word_has_hash_suffix(source);
            (suspicious_word_shape || command.bracket_command_name_needs_separator(source))
                .then_some(span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || TemplateBraceInCommand);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_double_brace_placeholders_in_command_position() {
        let source = "\
#!/bin/bash
\"$root/pkg/{{name}}/bin/{{cmd}}\" \"$@\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TemplateBraceInCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2]
        );
    }

    #[test]
    fn reports_other_suspicious_command_name_shapes() {
        let source = "\
#!/bin/sh
\"ERROR: missing first arg for name to docker_compose_version_test()\"
amoeba=\"\" [ \"${AMOEBA:-yes}\" = \"yes\" ]
>&2 \"Error: Not a readable file: '$CERTIFICATE_TO_ADD'\"
\"$PID exists but pid doesn't match pid of varnishd. please investigate.\"
\"$root/bin/{{\"
\"$root/bin/}}\"
\\[ x = y ]
+# added comment
printf# hi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TemplateBraceInCommand),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "\"ERROR: missing first arg for name to docker_compose_version_test()\"",
                "[",
                "\"Error: Not a readable file: '$CERTIFICATE_TO_ADD'\"",
                "\"$PID exists but pid doesn't match pid of varnishd. please investigate.\"",
                "\"$root/bin/{{\"",
                "\"$root/bin/}}\"",
                "\\[",
                "+#",
                "printf#",
            ]
        );
    }

    #[test]
    fn ignores_valid_quoted_commands_and_plain_tests() {
        let source = "\
#!/bin/bash
\"printf\" '%s\\n' hi
\"hello world\"
\"${loader:?}\"
\"/usr/bin/qemu-${machine}\"
\"$(printf cmd)\"
\"$(printf ')')\"
\"${cmd:-\\}}\"
command [ x = y ]
env FOO=1 [ x = y ]
>out command [ x = y ]
>out [ x = y ]
>out printf '%s\\n' hi
local_cmd() { :; }
local_cmd
percentual_change=$(( ((val2 - val1) * 100) / val1 )) # divisor_valid
[
  \"$1\" = yes
]
echo \"{{name}}\"
command \"{{tool}}\"
printf '%s\\n' \"$root/{{name}}/bin/{{cmd}}\"
echo hi > \"{{name}}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TemplateBraceInCommand),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_other_malformed_command_name_shapes_without_template_braces() {
        let source = "\
#!/bin/sh
+++ diff header
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::TemplateBraceInCommand),
        );

        assert!(diagnostics.is_empty());
    }
}
