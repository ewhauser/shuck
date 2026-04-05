use shuck_ast::{
    Assignment, AssignmentValue, BuiltinCommand, Command, CommandList, CompoundCommand,
    ConditionalExpr, DeclOperand, FunctionDef, Redirect, Span, Word, WordPart,
};

use crate::{Checker, Rule, Violation};

pub struct PipeToKill;

impl Violation for PipeToKill {
    fn rule() -> Rule {
        Rule::PipeToKill
    }

    fn message(&self) -> String {
        "piping data into `kill` has no effect".to_owned()
    }
}

pub fn pipe_to_kill(checker: &mut Checker) {
    let mut spans = Vec::new();
    collect_commands(&checker.ast().commands, checker.source(), &mut spans);

    for span in spans {
        checker.report(PipeToKill, span);
    }
}

fn collect_commands(commands: &[Command], source: &str, spans: &mut Vec<Span>) {
    for command in commands {
        collect_command(command, source, spans);
    }
}

fn collect_command(command: &Command, source: &str, spans: &mut Vec<Span>) {
    match command {
        Command::Simple(command) => {
            collect_assignments(&command.assignments, source, spans);
            collect_word(&command.name, source, spans);
            collect_words(&command.args, source, spans);
            collect_redirects(&command.redirects, source, spans);
        }
        Command::Builtin(command) => collect_builtin(command, source, spans),
        Command::Decl(command) => {
            collect_assignments(&command.assignments, source, spans);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_word(word, source, spans);
                    }
                    DeclOperand::Name(_) => {}
                    DeclOperand::Assignment(assignment) => {
                        collect_assignment(assignment, source, spans);
                    }
                }
            }
            collect_redirects(&command.redirects, source, spans);
        }
        Command::Pipeline(command) => {
            if command.commands.len() > 1
                && command
                    .commands
                    .last()
                    .is_some_and(|command| is_static_kill_command(command, source))
            {
                spans.push(command.span);
            }

            collect_commands(&command.commands, source, spans);
        }
        Command::List(CommandList { first, rest, .. }) => {
            collect_command(first, source, spans);
            for (_, command) in rest {
                collect_command(command, source, spans);
            }
        }
        Command::Compound(command, redirects) => {
            collect_compound(command, source, spans);
            collect_redirects(redirects, source, spans);
        }
        Command::Function(FunctionDef { body, .. }) => collect_command(body, source, spans),
    }
}

fn collect_builtin(command: &BuiltinCommand, source: &str, spans: &mut Vec<Span>) {
    match command {
        BuiltinCommand::Break(command) => {
            collect_assignments(&command.assignments, source, spans);
            if let Some(word) = &command.depth {
                collect_word(word, source, spans);
            }
            collect_words(&command.extra_args, source, spans);
            collect_redirects(&command.redirects, source, spans);
        }
        BuiltinCommand::Continue(command) => {
            collect_assignments(&command.assignments, source, spans);
            if let Some(word) = &command.depth {
                collect_word(word, source, spans);
            }
            collect_words(&command.extra_args, source, spans);
            collect_redirects(&command.redirects, source, spans);
        }
        BuiltinCommand::Return(command) => {
            collect_assignments(&command.assignments, source, spans);
            if let Some(word) = &command.code {
                collect_word(word, source, spans);
            }
            collect_words(&command.extra_args, source, spans);
            collect_redirects(&command.redirects, source, spans);
        }
        BuiltinCommand::Exit(command) => {
            collect_assignments(&command.assignments, source, spans);
            if let Some(word) = &command.code {
                collect_word(word, source, spans);
            }
            collect_words(&command.extra_args, source, spans);
            collect_redirects(&command.redirects, source, spans);
        }
    }
}

fn collect_compound(command: &CompoundCommand, source: &str, spans: &mut Vec<Span>) {
    match command {
        CompoundCommand::If(command) => {
            collect_commands(&command.condition, source, spans);
            collect_commands(&command.then_branch, source, spans);
            for (condition, body) in &command.elif_branches {
                collect_commands(condition, source, spans);
                collect_commands(body, source, spans);
            }
            if let Some(body) = &command.else_branch {
                collect_commands(body, source, spans);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                collect_words(words, source, spans);
            }
            collect_commands(&command.body, source, spans);
        }
        CompoundCommand::ArithmeticFor(command) => collect_commands(&command.body, source, spans),
        CompoundCommand::While(command) => {
            collect_commands(&command.condition, source, spans);
            collect_commands(&command.body, source, spans);
        }
        CompoundCommand::Until(command) => {
            collect_commands(&command.condition, source, spans);
            collect_commands(&command.body, source, spans);
        }
        CompoundCommand::Case(command) => {
            collect_word(&command.word, source, spans);
            for case in &command.cases {
                collect_words(&case.patterns, source, spans);
                collect_commands(&case.commands, source, spans);
            }
        }
        CompoundCommand::Select(command) => {
            collect_words(&command.words, source, spans);
            collect_commands(&command.body, source, spans);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            collect_commands(commands, source, spans);
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_command(command, source, spans);
            }
        }
        CompoundCommand::Conditional(command) => {
            collect_conditional_expr(&command.expression, source, spans);
        }
        CompoundCommand::Coproc(command) => collect_command(&command.body, source, spans),
    }
}

fn collect_assignments(assignments: &[Assignment], source: &str, spans: &mut Vec<Span>) {
    for assignment in assignments {
        collect_assignment(assignment, source, spans);
    }
}

fn collect_assignment(assignment: &Assignment, source: &str, spans: &mut Vec<Span>) {
    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_word(word, source, spans),
        AssignmentValue::Array(words) => collect_words(words, source, spans),
    }
}

fn collect_words(words: &[Word], source: &str, spans: &mut Vec<Span>) {
    for word in words {
        collect_word(word, source, spans);
    }
}

fn collect_word(word: &Word, source: &str, spans: &mut Vec<Span>) {
    collect_word_parts(&word.parts, source, spans);
}

fn collect_word_parts(parts: &[WordPart], source: &str, spans: &mut Vec<Span>) {
    for part in parts {
        match part {
            WordPart::CommandSubstitution(commands)
            | WordPart::ProcessSubstitution { commands, .. } => {
                collect_commands(commands, source, spans);
            }
            _ => {}
        }
    }
}

fn collect_conditional_expr(expression: &ConditionalExpr, source: &str, spans: &mut Vec<Span>) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_expr(&expr.left, source, spans);
            collect_conditional_expr(&expr.right, source, spans);
        }
        ConditionalExpr::Unary(expr) => collect_conditional_expr(&expr.expr, source, spans),
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_expr(&expr.expr, source, spans);
        }
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => collect_word(word, source, spans),
    }
}

fn collect_redirects(redirects: &[Redirect], source: &str, spans: &mut Vec<Span>) {
    for redirect in redirects {
        collect_word(&redirect.target, source, spans);
    }
}

fn is_static_kill_command(command: &Command, source: &str) -> bool {
    match command {
        Command::Simple(command) => {
            static_word_text(&command.name, source).as_deref() == Some("kill")
        }
        _ => false,
    }
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
