use shuck_ast::{Command, CompoundCommand, Span};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct FunctionParamsInSh;

impl Violation for FunctionParamsInSh {
    fn rule() -> Rule {
        Rule::FunctionParamsInSh
    }

    fn message(&self) -> String {
        "function definitions cannot take parameters in `sh` scripts".to_owned()
    }
}

pub fn function_params_in_sh(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let spans = checker
        .facts()
        .structural_commands()
        .collect::<Vec<_>>()
        .windows(2)
        .filter_map(|pair| function_parameter_syntax_span(pair, checker.source()))
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || FunctionParamsInSh);
}

fn function_parameter_syntax_span(pair: &[&crate::CommandFact<'_>], source: &str) -> Option<Span> {
    let [first, second] = pair else {
        return None;
    };
    let name = first.normalized().effective_or_literal_name()?;
    if !is_plausible_shell_function_name(name) || !first.normalized().body_args().is_empty() {
        return None;
    }
    if !matches!(first.command(), Command::Simple(_)) {
        return None;
    }
    let Command::Compound(CompoundCommand::Subshell(commands)) = second.command() else {
        return None;
    };
    if commands.is_empty() {
        return None;
    }
    if first.span().start.line != second.span().start.line {
        return None;
    }
    let tail = source.get(second.span().end.offset..)?;
    let tail = tail.trim_start_matches([' ', '\t', '\r', '\n']);
    if !matches!(tail.chars().next(), Some('{') | Some('(')) {
        return None;
    }
    let text = first.span().slice(source);
    let relative = text.find('(')?;
    let start = first.span().start.advanced_by(&text[..relative]);
    Some(Span::from_positions(start, start.advanced_by("(")))
}

fn is_plausible_shell_function_name(name: &str) -> bool {
    let Some(first) = name.chars().next() else {
        return false;
    };
    if !matches!(first, 'a'..='z' | 'A'..='Z' | '_') {
        return false;
    }
    if !name
        .chars()
        .all(|ch| matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-'))
    {
        return false;
    }
    !matches!(
        name,
        "!" | "{"
            | "}"
            | "if"
            | "then"
            | "else"
            | "elif"
            | "fi"
            | "do"
            | "done"
            | "case"
            | "esac"
            | "for"
            | "in"
            | "while"
            | "until"
            | "time"
            | "[["
            | "]]"
            | "function"
            | "select"
            | "coproc"
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_function_parameter_syntax_in_sh() {
        let source = "\
#!/bin/sh
f(x) { :; }
function g(y) { :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["(", "("]
        );
    }

    #[test]
    fn ignores_standard_function_definitions() {
        let source = "\
#!/bin/sh
f() { :; }
function g() { :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_function_parameter_syntax_with_subshell_body_in_sh() {
        let source = "\
#!/bin/sh
f(x) ( : )
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "(");
    }

    #[test]
    fn reports_function_parameter_syntax_when_body_starts_on_next_line() {
        let source = "\
#!/bin/sh
f(x)
{ :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "(");
    }

    #[test]
    fn reports_function_parameter_syntax_for_hyphenated_names() {
        let source = "\
#!/bin/sh
my-func(x) { :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "(");
    }

    #[test]
    fn ignores_empty_subshell_bodies() {
        let source = "\
#!/bin/sh
wget() { :; }
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FunctionParamsInSh));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_zsh_scripts() {
        let source = "#!/bin/zsh\nf(x) { :; }\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionParamsInSh).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
