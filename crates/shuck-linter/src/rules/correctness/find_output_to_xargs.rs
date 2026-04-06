use shuck_ast::{Command, Pipeline, Span, Word, WordPart};

use crate::rules::common::{
    command,
    query::{self, CommandWalkOptions},
};
use crate::{Checker, Rule, Violation};

pub struct FindOutputToXargs;

impl Violation for FindOutputToXargs {
    fn rule() -> Rule {
        Rule::FindOutputToXargs
    }

    fn message(&self) -> String {
        "raw `find` output piped to `xargs` can break on whitespace".to_owned()
    }
}

pub fn find_output_to_xargs(checker: &mut Checker) {
    let source = checker.source();
    let mut spans = Vec::new();

    query::walk_commands(
        &checker.ast().commands,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |command, _| {
            let Command::Pipeline(pipeline) = command else {
                return;
            };

            spans.extend(unsafe_find_to_xargs_spans(pipeline, source));
        },
    );

    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();

    for span in spans {
        checker.report(FindOutputToXargs, span);
    }
}

fn unsafe_find_to_xargs_spans(pipeline: &Pipeline, source: &str) -> Vec<Span> {
    pipeline
        .commands
        .windows(2)
        .filter_map(|pair| {
            let left = command::normalize_command(&pair[0], source);
            let right = command::normalize_command(&pair[1], source);

            if !left.effective_name_is("find") || !right.effective_name_is("xargs") {
                return None;
            }

            if find_uses_print0(left.body_args(), source)
                && xargs_uses_null_input(right.body_args(), source)
            {
                return None;
            }

            Some(left.body_span)
        })
        .collect()
}

fn find_uses_print0(args: &[&Word], source: &str) -> bool {
    args.iter()
        .filter_map(|word| static_word_text(word, source))
        .any(|arg| arg == "-print0")
}

fn xargs_uses_null_input(args: &[&Word], source: &str) -> bool {
    args.iter()
        .filter_map(|word| static_word_text(word, source))
        .any(|arg| {
            arg == "--null"
                || (arg.starts_with('-') && !arg.starts_with("--") && arg[1..].contains('0'))
        })
}

fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) => result.push_str(text.as_str(source, span)),
            _ => return None,
        }
    }
    Some(result)
}
