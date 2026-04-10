use shuck_ast::Span;

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct KeywordFunctionName;

impl Violation for KeywordFunctionName {
    fn rule() -> Rule {
        Rule::KeywordFunctionName
    }

    fn message(&self) -> String {
        "reserved word is used as a function name".to_owned()
    }
}

pub fn keyword_function_name(checker: &mut Checker) {
    if !matches!(
        checker.shell(),
        ShellDialect::Sh | ShellDialect::Bash | ShellDialect::Dash | ShellDialect::Ksh
    ) {
        return;
    }

    let shell = checker.shell();
    let spans = checker
        .facts()
        .function_headers()
        .iter()
        .flat_map(|header| {
            header.function().header.entries.iter().filter_map(move |entry| {
                let name = entry.static_name.as_ref()?.as_str();
                is_reserved_function_name(shell, name).then_some(entry.word.span)
            })
        })
        .collect::<Vec<Span>>();

    checker.report_all_dedup(spans, || KeywordFunctionName);
}

fn is_reserved_function_name(shell: ShellDialect, name: &str) -> bool {
    match shell {
        ShellDialect::Sh | ShellDialect::Dash => matches!(
            name,
            "if" | "then" | "else" | "elif" | "fi" | "do" | "done" | "case" | "esac" | "for"
                | "in" | "while" | "until" | "time"
        ),
        ShellDialect::Bash | ShellDialect::Ksh => matches!(
            name,
            "if" | "then" | "else" | "elif" | "fi" | "do" | "done" | "case" | "esac" | "for"
                | "in" | "while" | "until" | "time" | "select" | "coproc"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet_at_path;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_reserved_words_as_function_names() {
        let source = "\
#!/bin/sh
function for { :; }
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::KeywordFunctionName),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::KeywordFunctionName);
    }

    #[test]
    fn ignores_non_reserved_function_names() {
        let source = "\
#!/bin/sh
name() { :; }
";
        let diagnostics = test_snippet_at_path(
            std::path::Path::new("/tmp/project/main.sh"),
            source,
            &LinterSettings::for_rule(Rule::KeywordFunctionName),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
