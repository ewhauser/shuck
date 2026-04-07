use shuck_ast::{BinaryOp, Command, Span, Stmt, Word};

use crate::rules::common::{
    command,
    query::{self, CommandWalkOptions},
    word::static_word_text,
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

    query::walk_commands(
        &checker.ast().body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
        &mut |visit| {
            let command = visit.command;
            let Some(pipeline) = pipeline_segments(command) else {
                return;
            };

            for span in unsafe_find_to_xargs_spans(&pipeline, source) {
                checker.report_dedup(FindOutputToXargs, span);
            }
        },
    );
}

fn unsafe_find_to_xargs_spans(pipeline: &[&Stmt], source: &str) -> Vec<Span> {
    pipeline
        .windows(2)
        .filter_map(|pair| {
            let left = command::normalize_command(&pair[0].command, source);
            let right = command::normalize_command(&pair[1].command, source);

            if !left.effective_name_is("find") || !right.effective_name_is("xargs") {
                return None;
            }

            if find_uses_print0(left.body_args(), source)
                && xargs_uses_null_input(right.body_args(), source)
            {
                return None;
            }

            Some(find_command_span(pair[0], left))
        })
        .collect()
}

fn find_command_span(command: &Stmt, normalized: command::NormalizedCommand<'_>) -> Span {
    match &command.command {
        Command::Simple(simple) => {
            let end = command
                .redirects
                .last()
                .map(|redirect| redirect.span.end)
                .or_else(|| simple.args.last().map(|word| word.span.end))
                .unwrap_or(simple.name.span.end);
            Span::from_positions(normalized.body_span.start, end)
        }
        _ => normalized.body_span,
    }
}

fn pipeline_segments(command: &Command) -> Option<Vec<&Stmt>> {
    let Command::Binary(command) = command else {
        return None;
    };
    if !matches!(command.op, BinaryOp::Pipe | BinaryOp::PipeAll) {
        return None;
    }

    let mut segments = Vec::new();
    collect_pipeline_segments(command, &mut segments);
    Some(segments)
}

fn collect_pipeline_segments<'a>(command: &'a shuck_ast::BinaryCommand, segments: &mut Vec<&'a Stmt>) {
    match &command.left.command {
        Command::Binary(left) if matches!(left.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_segments(left, segments);
        }
        _ => segments.push(&command.left),
    }

    match &command.right.command {
        Command::Binary(right) if matches!(right.op, BinaryOp::Pipe | BinaryOp::PipeAll) => {
            collect_pipeline_segments(right, segments);
        }
        _ => segments.push(&command.right),
    }
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

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_effective_find_command_name() {
        let source = "command find . -type f | xargs wc -l\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "find . -type f");
    }

    #[test]
    fn anchors_on_multiline_find_segment_before_pipe() {
        let source = "find . -type f \\\n  -name '*.txt' | xargs rm\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::FindOutputToXargs));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(
            diagnostics[0].span.slice(source),
            "find . -type f \\\n  -name '*.txt'"
        );
    }
}
