use crate::{Checker, ExpansionContext, Rule, Violation, WordFactContext};

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
            let text = span.slice(source);
            let trailing_literal_char = facts
                .word_fact(
                    span,
                    WordFactContext::Expansion(ExpansionContext::CommandName),
                )
                .and_then(|word| word.trailing_literal_char());
            (contains_template_placeholder(text)
                || quoted_command_name_has_suspicious_ending(text, trailing_literal_char)
                || unquoted_command_name_has_hash_suffix(text)
                || bracket_command_name_needs_separator(command, span, source))
            .then_some(span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || TemplateBraceInCommand);
}

fn contains_template_placeholder(text: &str) -> bool {
    let Some(start) = text.find("{{") else {
        return false;
    };
    text[start + 2..].contains("}}")
}

fn quoted_command_name_has_suspicious_ending(
    text: &str,
    trailing_literal_char: Option<char>,
) -> bool {
    let Some(inner) = strip_matching_quotes(text) else {
        return false;
    };

    let Some(ch) = trailing_literal_char.or_else(|| inner.chars().next_back()) else {
        return false;
    };
    if !is_suspicious_command_trailer(ch) {
        return false;
    }
    if trailing_literal_char.is_some() {
        return true;
    }

    match ch {
        '}' => !inner_ends_with_parameter_expansion(inner),
        ')' => !inner_ends_with_command_substitution(inner),
        _ => true,
    }
}

fn strip_matching_quotes(text: &str) -> Option<&str> {
    if text.len() < 2 {
        return None;
    }

    match (
        text.as_bytes().first().copied(),
        text.as_bytes().last().copied(),
    ) {
        (Some(b'"'), Some(b'"')) | (Some(b'\''), Some(b'\'')) => Some(&text[1..text.len() - 1]),
        _ => None,
    }
}

fn is_suspicious_command_trailer(ch: char) -> bool {
    matches!(
        ch,
        '.' | ',' | '#' | '[' | ']' | '(' | ')' | '{' | '}' | '\''
    )
}

fn inner_ends_with_parameter_expansion(inner: &str) -> bool {
    if !inner.ends_with('}') {
        return false;
    }

    let bytes = inner.as_bytes();
    let mut depth = 1usize;
    let mut index = bytes.len() - 1;

    while index > 0 {
        index -= 1;
        match bytes[index] {
            b'}' => depth += 1,
            b'{' => {
                depth -= 1;
                if depth == 0 {
                    return index > 0 && bytes[index - 1] == b'$';
                }
            }
            _ => {}
        }
    }

    false
}

fn inner_ends_with_command_substitution(inner: &str) -> bool {
    if !inner.ends_with(')') {
        return false;
    }

    let bytes = inner.as_bytes();
    let mut depth = 1usize;
    let mut index = bytes.len() - 1;

    while index > 0 {
        index -= 1;
        match bytes[index] {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    return index > 0 && bytes[index - 1] == b'$';
                }
            }
            _ => {}
        }
    }

    false
}

fn unquoted_command_name_has_hash_suffix(text: &str) -> bool {
    text != "#" && text.ends_with('#')
}

fn bracket_command_name_needs_separator(
    command: &crate::CommandFact<'_>,
    span: shuck_ast::Span,
    source: &str,
) -> bool {
    if command.literal_name() != Some("[") {
        return false;
    }

    let raw = span.slice(source);
    raw != "[" || command.span().start.offset < span.start.offset
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
command [ x = y ]
env FOO=1 [ x = y ]
>out command [ x = y ]
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
