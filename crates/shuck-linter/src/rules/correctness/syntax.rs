use shuck_ast::{
    Assignment, AssignmentValue, BuiltinCommand, Command, CommandList, CompoundCommand,
    ConditionalExpr, DeclOperand, FunctionDef, Redirect, SimpleCommand, TextSize, Word, WordPart,
};
use shuck_indexer::{Indexer, RegionKind};

#[derive(Debug, Clone, Copy, Default)]
pub struct WalkContext {
    pub loop_depth: usize,
}

pub fn walk_commands(commands: &[Command], visitor: &mut impl FnMut(&Command, WalkContext)) {
    Walker { visitor }.walk_commands(commands, WalkContext::default());
}

pub fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) => result.push_str(text.as_str(source, span)),
            _ => return None,
        }
    }
    Some(result)
}

pub fn is_simple_command_named(command: &Command, source: &str, name: &str) -> bool {
    match command {
        Command::Simple(command) => {
            static_word_text(&command.name, source).as_deref() == Some(name)
        }
        _ => false,
    }
}

pub fn simple_test_operands<'a>(command: &'a SimpleCommand, source: &str) -> Option<&'a [Word]> {
    let name = static_word_text(&command.name, source)?;
    match name.as_str() {
        "[" => {
            let (closing_bracket, operands) = command.args.split_last()?;
            (static_word_text(closing_bracket, source).as_deref() == Some("]")).then_some(operands)
        }
        "test" => Some(&command.args),
        _ => None,
    }
}

pub fn word_has_expansion(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| !matches!(part, WordPart::Literal(_)))
}

pub fn word_is_double_quoted(indexer: &Indexer, word: &Word) -> bool {
    let span = word.part_span(0).unwrap_or(word.span);
    indexer
        .region_index()
        .region_at(TextSize::new(span.start.offset as u32))
        == Some(RegionKind::DoubleQuoted)
}

struct Walker<'a, F> {
    visitor: &'a mut F,
}

impl<F: FnMut(&Command, WalkContext)> Walker<'_, F> {
    fn walk_commands(&mut self, commands: &[Command], context: WalkContext) {
        for command in commands {
            self.walk_command(command, context);
        }
    }

    fn walk_command(&mut self, command: &Command, context: WalkContext) {
        (self.visitor)(command, context);

        match command {
            Command::Simple(command) => {
                self.walk_assignments(&command.assignments, context);
                self.walk_word(&command.name, context);
                self.walk_words(&command.args, context);
                self.walk_redirects(&command.redirects, context);
            }
            Command::Builtin(command) => self.walk_builtin(command, context),
            Command::Decl(command) => {
                self.walk_assignments(&command.assignments, context);
                for operand in &command.operands {
                    match operand {
                        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                            self.walk_word(word, context);
                        }
                        DeclOperand::Name(_) => {}
                        DeclOperand::Assignment(assignment) => {
                            self.walk_assignment(assignment, context);
                        }
                    }
                }
                self.walk_redirects(&command.redirects, context);
            }
            Command::Pipeline(command) => self.walk_commands(&command.commands, context),
            Command::List(CommandList { first, rest, .. }) => {
                self.walk_command(first, context);
                for (_, command) in rest {
                    self.walk_command(command, context);
                }
            }
            Command::Compound(command, redirects) => {
                self.walk_compound(command, context);
                self.walk_redirects(redirects, context);
            }
            Command::Function(FunctionDef { body, .. }) => self.walk_command(body, context),
        }
    }

    fn walk_builtin(&mut self, command: &BuiltinCommand, context: WalkContext) {
        match command {
            BuiltinCommand::Break(command) => {
                self.walk_assignments(&command.assignments, context);
                if let Some(word) = &command.depth {
                    self.walk_word(word, context);
                }
                self.walk_words(&command.extra_args, context);
                self.walk_redirects(&command.redirects, context);
            }
            BuiltinCommand::Continue(command) => {
                self.walk_assignments(&command.assignments, context);
                if let Some(word) = &command.depth {
                    self.walk_word(word, context);
                }
                self.walk_words(&command.extra_args, context);
                self.walk_redirects(&command.redirects, context);
            }
            BuiltinCommand::Return(command) => {
                self.walk_assignments(&command.assignments, context);
                if let Some(word) = &command.code {
                    self.walk_word(word, context);
                }
                self.walk_words(&command.extra_args, context);
                self.walk_redirects(&command.redirects, context);
            }
            BuiltinCommand::Exit(command) => {
                self.walk_assignments(&command.assignments, context);
                if let Some(word) = &command.code {
                    self.walk_word(word, context);
                }
                self.walk_words(&command.extra_args, context);
                self.walk_redirects(&command.redirects, context);
            }
        }
    }

    fn walk_compound(&mut self, command: &CompoundCommand, context: WalkContext) {
        match command {
            CompoundCommand::If(command) => {
                self.walk_commands(&command.condition, context);
                self.walk_commands(&command.then_branch, context);
                for (condition, body) in &command.elif_branches {
                    self.walk_commands(condition, context);
                    self.walk_commands(body, context);
                }
                if let Some(body) = &command.else_branch {
                    self.walk_commands(body, context);
                }
            }
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    self.walk_words(words, context);
                }
                self.walk_commands(
                    &command.body,
                    WalkContext {
                        loop_depth: context.loop_depth + 1,
                    },
                );
            }
            CompoundCommand::ArithmeticFor(command) => self.walk_commands(
                &command.body,
                WalkContext {
                    loop_depth: context.loop_depth + 1,
                },
            ),
            CompoundCommand::While(command) => {
                let loop_context = WalkContext {
                    loop_depth: context.loop_depth + 1,
                };
                self.walk_commands(&command.condition, loop_context);
                self.walk_commands(&command.body, loop_context);
            }
            CompoundCommand::Until(command) => {
                let loop_context = WalkContext {
                    loop_depth: context.loop_depth + 1,
                };
                self.walk_commands(&command.condition, loop_context);
                self.walk_commands(&command.body, loop_context);
            }
            CompoundCommand::Case(command) => {
                self.walk_word(&command.word, context);
                for case in &command.cases {
                    self.walk_words(&case.patterns, context);
                    self.walk_commands(&case.commands, context);
                }
            }
            CompoundCommand::Select(command) => {
                self.walk_words(&command.words, context);
                self.walk_commands(
                    &command.body,
                    WalkContext {
                        loop_depth: context.loop_depth + 1,
                    },
                );
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                self.walk_commands(commands, context);
            }
            CompoundCommand::Arithmetic(_) => {}
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.walk_command(command, context);
                }
            }
            CompoundCommand::Conditional(command) => {
                self.walk_conditional_expr(&command.expression, context);
            }
            CompoundCommand::Coproc(command) => self.walk_command(&command.body, context),
        }
    }

    fn walk_assignments(&mut self, assignments: &[Assignment], context: WalkContext) {
        for assignment in assignments {
            self.walk_assignment(assignment, context);
        }
    }

    fn walk_assignment(&mut self, assignment: &Assignment, context: WalkContext) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.walk_word(word, context),
            AssignmentValue::Array(words) => self.walk_words(words, context),
        }
    }

    fn walk_words(&mut self, words: &[Word], context: WalkContext) {
        for word in words {
            self.walk_word(word, context);
        }
    }

    fn walk_word(&mut self, word: &Word, context: WalkContext) {
        for part in &word.parts {
            match part {
                WordPart::CommandSubstitution(commands)
                | WordPart::ProcessSubstitution { commands, .. } => {
                    self.walk_commands(commands, context);
                }
                _ => {}
            }
        }
    }

    fn walk_conditional_expr(&mut self, expression: &ConditionalExpr, context: WalkContext) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.walk_conditional_expr(&expr.left, context);
                self.walk_conditional_expr(&expr.right, context);
            }
            ConditionalExpr::Unary(expr) => self.walk_conditional_expr(&expr.expr, context),
            ConditionalExpr::Parenthesized(expr) => self.walk_conditional_expr(&expr.expr, context),
            ConditionalExpr::Word(word)
            | ConditionalExpr::Pattern(word)
            | ConditionalExpr::Regex(word) => self.walk_word(word, context),
        }
    }

    fn walk_redirects(&mut self, redirects: &[Redirect], context: WalkContext) {
        for redirect in redirects {
            self.walk_word(&redirect.target, context);
        }
    }
}
