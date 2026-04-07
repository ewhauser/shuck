use shuck_ast::Word;

use crate::rules::common::{
    command,
    query::{self, CommandWalkOptions},
    word::{classify_word, static_word_text},
};
use crate::{Checker, Rule, Violation};

pub struct PrintfFormatVariable;

impl Violation for PrintfFormatVariable {
    fn rule() -> Rule {
        Rule::PrintfFormatVariable
    }

    fn message(&self) -> String {
        "keep `printf` format strings literal instead of expanding them from variables".to_owned()
    }
}

pub fn printf_format_variable(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        checker.source(),
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let normalized = command::normalize_command(command, source);
            if !normalized.effective_name_is("printf") {
                return;
            }

            let Some(format_word) = printf_format_word(normalized.body_args(), source) else {
                return;
            };

            if !classify_word(format_word, source).is_fixed_literal() {
                spans.push(format_word.span);
            }
        },
    );

    for span in spans {
        checker.report(PrintfFormatVariable, span);
    }
}

fn printf_format_word<'a>(args: &[&'a Word], source: &str) -> Option<&'a Word> {
    let mut index = 0usize;

    if static_word_text(args.get(index)?, source).as_deref() == Some("--") {
        index += 1;
    }

    if let Some(option) = args
        .get(index)
        .and_then(|word| static_word_text(word, source))
    {
        if option == "-v" {
            index += 2;
        } else if option.starts_with("-v") && option.len() > 2 {
            index += 1;
        }
    }

    args.get(index).copied()
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_runtime_supplied_formats_and_skips_fixed_literals() {
        let source = "printf '%s\\n' value\nprintf \"$fmt\" value\nprintf \"$(echo %s)\" value\n";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PrintfFormatVariable),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.start.line)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }
}
