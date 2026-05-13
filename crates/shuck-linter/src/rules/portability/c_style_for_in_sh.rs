use shuck_ast::{Command, CompoundCommand, Span};

use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, ShellDialect, Violation};

pub struct CStyleForInSh;

impl Violation for CStyleForInSh {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Sometimes;

    fn rule() -> Rule {
        Rule::CStyleForInSh
    }

    fn message(&self) -> String {
        "C-style `for ((...))` loops are not portable in `sh` scripts".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("rewrite as a portable while loop".to_owned())
    }
}

pub fn c_style_for_in_sh(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Sh | ShellDialect::Dash) {
        return;
    }

    let diagnostics = checker
        .facts()
        .commands()
        .iter()
        .filter_map(|fact| match fact.command() {
            Command::Compound(CompoundCommand::ArithmeticFor(command)) => {
                let span = keyword_span(fact.span_in_source(checker.source()), "for");
                let diagnostic = Diagnostic::new(CStyleForInSh, span);
                Some(
                    match c_style_for_fix(
                        command,
                        fact.span_in_source(checker.source()),
                        checker.source(),
                    ) {
                        Some(fix) => diagnostic.with_fix(fix),
                        None => diagnostic,
                    },
                )
            }
            Command::Simple(_)
            | Command::Builtin(_)
            | Command::Decl(_)
            | Command::Binary(_)
            | Command::Compound(_)
            | Command::Function(_)
            | Command::AnonymousFunction(_) => None,
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic(diagnostic);
    }
}

fn keyword_span(span: Span, keyword: &str) -> Span {
    Span::from_positions(span.start, span.start.advanced_by(keyword))
}

fn c_style_for_fix(
    command: &shuck_ast::ArithmeticForCommand,
    replacement_span: Span,
    source: &str,
) -> Option<Fix> {
    let init = command.init_span.map(|span| span.slice(source).trim());
    let condition = command
        .condition_span
        .map(|span| span.slice(source).trim())
        .filter(|condition| !condition.is_empty())
        .unwrap_or("1");
    let step = command.step_span.map(|span| span.slice(source).trim());
    let body = command.body.span.slice(source).trim_end();
    if body.is_empty() {
        return None;
    }

    let indent = line_indent_before(source, replacement_span.start.offset);
    let child_indent = format!("{indent}  ");
    let mut replacement = String::new();
    if let Some(init) = init.filter(|init| !init.is_empty()) {
        replacement.push_str(&format!("{indent}: \"$(({init}))\"\n"));
    }
    replacement.push_str(&format!(
        "{indent}while [ \"$(({condition}))\" -ne 0 ]; do\n"
    ));
    push_indented_body(&mut replacement, body, &child_indent);
    if let Some(step) = step.filter(|step| !step.is_empty()) {
        replacement.push_str(&format!(
            "{child_indent}: \"$(({}))\"\n",
            portable_arithmetic_update(step)
        ));
    }
    replacement.push_str(&format!("{indent}done"));

    Some(Fix::unsafe_edit(Edit::replacement(
        replacement,
        replacement_span,
    )))
}

fn push_indented_body(replacement: &mut String, body: &str, child_indent: &str) {
    for line in body.lines() {
        if line.trim().is_empty() {
            replacement.push('\n');
        } else {
            replacement.push_str(child_indent);
            replacement.push_str(line.trim_start());
            replacement.push('\n');
        }
    }
}

fn portable_arithmetic_update(expression: &str) -> String {
    if let Some(name) = expression
        .strip_suffix("++")
        .filter(|name| is_arithmetic_name(name))
    {
        return format!("{name} = {name} + 1");
    }
    if let Some(name) = expression
        .strip_suffix("--")
        .filter(|name| is_arithmetic_name(name))
    {
        return format!("{name} = {name} - 1");
    }
    if let Some(name) = expression
        .strip_prefix("++")
        .filter(|name| is_arithmetic_name(name))
    {
        return format!("{name} = {name} + 1");
    }
    if let Some(name) = expression
        .strip_prefix("--")
        .filter(|name| is_arithmetic_name(name))
    {
        return format!("{name} = {name} - 1");
    }
    expression.to_owned()
}

fn is_arithmetic_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn line_indent_before(source: &str, offset: usize) -> &str {
    let line_start = source[..offset].rfind('\n').map_or(0, |offset| offset + 1);
    let line_prefix = &source[line_start..offset];
    let indent_len = line_prefix
        .bytes()
        .take_while(|byte| matches!(byte, b' ' | b'\t'))
        .count();
    &line_prefix[..indent_len]
}

#[cfg(test)]
mod tests {
    use crate::test::{test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, ShellDialect};

    #[test]
    fn anchors_on_for_keyword_only() {
        let source = "#!/bin/sh\nfor ((i = 0; i < 5; i++)); do echo \"$i\"; done\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CStyleForInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "for");
    }

    #[test]
    fn ignores_bash_scripts() {
        let source = "#!/bin/bash\nfor ((i = 0; i < 5; i++)); do echo \"$i\"; done\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CStyleForInSh).with_shell(ShellDialect::Bash),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_rewrite_c_style_for_loop() {
        let source = "\
#!/bin/sh
for ((i = 0; i < 3; i++)); do
  echo \"$i\"
done
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CStyleForInSh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/sh
: \"$((i = 0))\"
while [ \"$((i < 3))\" -ne 0 ]; do
  echo \"$i\"
  : \"$((i = i + 1))\"
done
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_without_copying_inline_prefix_as_indent() {
        let source =
            "#!/bin/sh\nif ready; then for ((i = 0; i < 1; i++)); do echo \"$i\"; done; fi\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::CStyleForInSh),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 1);
        assert_eq!(
            result.fixed_source,
            "#!/bin/sh\nif ready; then : \"$((i = 0))\"\nwhile [ \"$((i < 1))\" -ne 0 ]; do\n  echo \"$i\";\n  : \"$((i = i + 1))\"\ndone; fi\n"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }
}
