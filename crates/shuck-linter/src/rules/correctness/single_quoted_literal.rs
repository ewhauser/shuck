use shuck_ast::{
    Assignment, AssignmentValue, BuiltinCommand, Command, CommandList, CompoundCommand,
    ConditionalExpr, DeclOperand, FunctionDef, Redirect, Span, TextSize, Word, WordPart,
};
use shuck_indexer::{Indexer, RegionKind};

use crate::{Checker, Rule, Violation};

pub struct SingleQuotedLiteral;

impl Violation for SingleQuotedLiteral {
    fn rule() -> Rule {
        Rule::SingleQuotedLiteral
    }

    fn message(&self) -> String {
        "shell expansion inside single quotes stays literal".to_owned()
    }
}

pub fn single_quoted_literal(checker: &mut Checker) {
    let mut spans = Vec::new();
    collect_commands(
        &checker.ast().commands,
        checker.indexer(),
        checker.source(),
        &mut spans,
    );

    for span in spans {
        checker.report(SingleQuotedLiteral, span);
    }
}

fn collect_commands(commands: &[Command], indexer: &Indexer, source: &str, spans: &mut Vec<Span>) {
    for command in commands {
        collect_command(command, indexer, source, spans);
    }
}

fn collect_command(command: &Command, indexer: &Indexer, source: &str, spans: &mut Vec<Span>) {
    match command {
        Command::Simple(command) => {
            collect_assignments(&command.assignments, indexer, source, spans);
            collect_word(&command.name, indexer, source, spans);
            collect_words(&command.args, indexer, source, spans);
            collect_redirects(&command.redirects, indexer, source, spans);
        }
        Command::Builtin(command) => collect_builtin(command, indexer, source, spans),
        Command::Decl(command) => {
            collect_assignments(&command.assignments, indexer, source, spans);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_word(word, indexer, source, spans);
                    }
                    DeclOperand::Name(_) => {}
                    DeclOperand::Assignment(assignment) => {
                        collect_assignment(assignment, indexer, source, spans);
                    }
                }
            }
            collect_redirects(&command.redirects, indexer, source, spans);
        }
        Command::Pipeline(command) => collect_commands(&command.commands, indexer, source, spans),
        Command::List(CommandList { first, rest, .. }) => {
            collect_command(first, indexer, source, spans);
            for (_, command) in rest {
                collect_command(command, indexer, source, spans);
            }
        }
        Command::Compound(command, redirects) => {
            collect_compound(command, indexer, source, spans);
            collect_redirects(redirects, indexer, source, spans);
        }
        Command::Function(FunctionDef { body, .. }) => {
            collect_command(body, indexer, source, spans)
        }
    }
}

fn collect_builtin(
    command: &BuiltinCommand,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        BuiltinCommand::Break(command) => {
            collect_assignments(&command.assignments, indexer, source, spans);
            if let Some(word) = &command.depth {
                collect_word(word, indexer, source, spans);
            }
            collect_words(&command.extra_args, indexer, source, spans);
            collect_redirects(&command.redirects, indexer, source, spans);
        }
        BuiltinCommand::Continue(command) => {
            collect_assignments(&command.assignments, indexer, source, spans);
            if let Some(word) = &command.depth {
                collect_word(word, indexer, source, spans);
            }
            collect_words(&command.extra_args, indexer, source, spans);
            collect_redirects(&command.redirects, indexer, source, spans);
        }
        BuiltinCommand::Return(command) => {
            collect_assignments(&command.assignments, indexer, source, spans);
            if let Some(word) = &command.code {
                collect_word(word, indexer, source, spans);
            }
            collect_words(&command.extra_args, indexer, source, spans);
            collect_redirects(&command.redirects, indexer, source, spans);
        }
        BuiltinCommand::Exit(command) => {
            collect_assignments(&command.assignments, indexer, source, spans);
            if let Some(word) = &command.code {
                collect_word(word, indexer, source, spans);
            }
            collect_words(&command.extra_args, indexer, source, spans);
            collect_redirects(&command.redirects, indexer, source, spans);
        }
    }
}

fn collect_compound(
    command: &CompoundCommand,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        CompoundCommand::If(command) => {
            collect_commands(&command.condition, indexer, source, spans);
            collect_commands(&command.then_branch, indexer, source, spans);
            for (condition, body) in &command.elif_branches {
                collect_commands(condition, indexer, source, spans);
                collect_commands(body, indexer, source, spans);
            }
            if let Some(body) = &command.else_branch {
                collect_commands(body, indexer, source, spans);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                collect_words(words, indexer, source, spans);
            }
            collect_commands(&command.body, indexer, source, spans);
        }
        CompoundCommand::ArithmeticFor(command) => {
            collect_commands(&command.body, indexer, source, spans);
        }
        CompoundCommand::While(command) => {
            collect_commands(&command.condition, indexer, source, spans);
            collect_commands(&command.body, indexer, source, spans);
        }
        CompoundCommand::Until(command) => {
            collect_commands(&command.condition, indexer, source, spans);
            collect_commands(&command.body, indexer, source, spans);
        }
        CompoundCommand::Case(command) => {
            collect_word(&command.word, indexer, source, spans);
            for case in &command.cases {
                collect_words(&case.patterns, indexer, source, spans);
                collect_commands(&case.commands, indexer, source, spans);
            }
        }
        CompoundCommand::Select(command) => {
            collect_words(&command.words, indexer, source, spans);
            collect_commands(&command.body, indexer, source, spans);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            collect_commands(commands, indexer, source, spans);
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_command(command, indexer, source, spans);
            }
        }
        CompoundCommand::Conditional(command) => {
            collect_conditional_expr(&command.expression, indexer, source, spans);
        }
        CompoundCommand::Coproc(command) => collect_command(&command.body, indexer, source, spans),
    }
}

fn collect_assignments(
    assignments: &[Assignment],
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for assignment in assignments {
        collect_assignment(assignment, indexer, source, spans);
    }
}

fn collect_assignment(
    assignment: &Assignment,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_word(word, indexer, source, spans),
        AssignmentValue::Array(words) => collect_words(words, indexer, source, spans),
    }
}

fn collect_words(words: &[Word], indexer: &Indexer, source: &str, spans: &mut Vec<Span>) {
    for word in words {
        collect_word(word, indexer, source, spans);
    }
}

fn collect_word(word: &Word, indexer: &Indexer, source: &str, spans: &mut Vec<Span>) {
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text)
                if is_single_quoted(indexer, span)
                    && contains_expansion_like_text(text.as_str(source, span)) =>
            {
                spans.push(span);
            }
            WordPart::CommandSubstitution(commands)
            | WordPart::ProcessSubstitution { commands, .. } => {
                collect_commands(commands, indexer, source, spans);
            }
            _ => {}
        }
    }
}

fn collect_redirects(
    redirects: &[Redirect],
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for redirect in redirects {
        collect_word(&redirect.target, indexer, source, spans);
    }
}

fn collect_conditional_expr(
    expression: &ConditionalExpr,
    indexer: &Indexer,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_expr(&expr.left, indexer, source, spans);
            collect_conditional_expr(&expr.right, indexer, source, spans);
        }
        ConditionalExpr::Unary(expr) => {
            collect_conditional_expr(&expr.expr, indexer, source, spans);
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_expr(&expr.expr, indexer, source, spans);
        }
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => collect_word(word, indexer, source, spans),
    }
}

fn is_single_quoted(indexer: &Indexer, span: Span) -> bool {
    indexer
        .region_index()
        .region_at(TextSize::new(span.start.offset as u32))
        == Some(RegionKind::SingleQuoted)
}

fn contains_expansion_like_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index + 1 < bytes.len() {
        if bytes[index] == b'$'
            && matches!(
                bytes[index + 1],
                b'{' | b'('
                    | b'_'
                    | b'a'..=b'z'
                    | b'A'..=b'Z'
                    | b'0'..=b'9'
                    | b'#'
                    | b'@'
                    | b'*'
                    | b'?'
                    | b'-'
                    | b'!'
                    | b'$'
            )
        {
            return true;
        }

        index += 1;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::contains_expansion_like_text;

    #[test]
    fn detects_variable_like_sequences() {
        assert!(contains_expansion_like_text("$HOME"));
        assert!(contains_expansion_like_text("${name:-default}"));
        assert!(contains_expansion_like_text("$(pwd)"));
        assert!(contains_expansion_like_text("$1"));
    }

    #[test]
    fn ignores_plain_text_without_expansions() {
        assert!(!contains_expansion_like_text("hello world"));
        assert!(!contains_expansion_like_text("$"));
        assert!(!contains_expansion_like_text("cost is USD 5"));
    }
}
