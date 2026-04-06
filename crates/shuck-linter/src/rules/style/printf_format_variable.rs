use shuck_ast::Word;

use crate::rules::common::{
    command,
    query::{self, CommandWalkOptions},
};
use crate::{Checker, Rule, Violation};

use super::syntax::static_word_text;

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

            if static_word_text(format_word, source).is_none() {
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
