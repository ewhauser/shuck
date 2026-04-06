use shuck_ast::{
    Assignment, AssignmentValue, BuiltinCommand, Command, CommandList, CompoundCommand,
    ConditionalExpr, DeclOperand, FunctionDef, Redirect, Word, WordPart,
};

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

pub fn word_contains_command_substitution(word: &Word) -> bool {
    word.parts
        .iter()
        .any(|part| matches!(part, WordPart::CommandSubstitution(_)))
}

pub fn word_is_plain_command_substitution(word: &Word) -> bool {
    matches!(word.parts.as_slice(), [WordPart::CommandSubstitution(_)])
}

pub fn visit_argument_words(command: &Command, mut visitor: impl FnMut(&Word)) {
    match command {
        Command::Simple(command) => {
            for word in &command.args {
                visitor(word);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
            BuiltinCommand::Continue(command) => {
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
            BuiltinCommand::Return(command) => {
                if let Some(word) = &command.code {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
            BuiltinCommand::Exit(command) => {
                if let Some(word) = &command.code {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
            }
        },
        _ => {}
    }
}

pub fn walk_commands(commands: &[Command], visitor: &mut impl FnMut(&Command)) {
    Walker { visitor }.walk_commands(commands);
}

pub fn walk_words(commands: &[Command], visitor: &mut impl FnMut(&Word)) {
    WordWalker { visitor }.walk_commands(commands);
}

struct Walker<'a, F> {
    visitor: &'a mut F,
}

impl<F: FnMut(&Command)> Walker<'_, F> {
    fn walk_commands(&mut self, commands: &[Command]) {
        for command in commands {
            self.walk_command(command);
        }
    }

    fn walk_command(&mut self, command: &Command) {
        (self.visitor)(command);

        match command {
            Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {}
            Command::Pipeline(command) => self.walk_commands(&command.commands),
            Command::List(CommandList { first, rest, .. }) => {
                self.walk_command(first);
                for (_, command) in rest {
                    self.walk_command(command);
                }
            }
            Command::Compound(command, _) => self.walk_compound(command),
            Command::Function(FunctionDef { body, .. }) => self.walk_command(body),
        }
    }

    fn walk_compound(&mut self, command: &CompoundCommand) {
        match command {
            CompoundCommand::If(command) => {
                self.walk_commands(&command.condition);
                self.walk_commands(&command.then_branch);
                for (condition, body) in &command.elif_branches {
                    self.walk_commands(condition);
                    self.walk_commands(body);
                }
                if let Some(body) = &command.else_branch {
                    self.walk_commands(body);
                }
            }
            CompoundCommand::For(command) => self.walk_commands(&command.body),
            CompoundCommand::ArithmeticFor(command) => self.walk_commands(&command.body),
            CompoundCommand::While(command) => {
                self.walk_commands(&command.condition);
                self.walk_commands(&command.body);
            }
            CompoundCommand::Until(command) => {
                self.walk_commands(&command.condition);
                self.walk_commands(&command.body);
            }
            CompoundCommand::Case(command) => {
                for case in &command.cases {
                    self.walk_commands(&case.commands);
                }
            }
            CompoundCommand::Select(command) => self.walk_commands(&command.body),
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                self.walk_commands(commands);
            }
            CompoundCommand::Arithmetic(_) => {}
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.walk_command(command);
                }
            }
            CompoundCommand::Conditional(_) => {}
            CompoundCommand::Coproc(command) => self.walk_command(&command.body),
        }
    }
}

struct WordWalker<'a, F> {
    visitor: &'a mut F,
}

impl<F: FnMut(&Word)> WordWalker<'_, F> {
    fn walk_commands(&mut self, commands: &[Command]) {
        for command in commands {
            self.walk_command(command);
        }
    }

    fn walk_command(&mut self, command: &Command) {
        match command {
            Command::Simple(command) => {
                self.walk_assignments(&command.assignments);
                self.walk_word(&command.name);
                self.walk_words(&command.args);
                self.walk_redirects(&command.redirects);
            }
            Command::Builtin(command) => self.walk_builtin(command),
            Command::Decl(command) => {
                self.walk_assignments(&command.assignments);
                for operand in &command.operands {
                    match operand {
                        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                            self.walk_word(word)
                        }
                        DeclOperand::Name(_) => {}
                        DeclOperand::Assignment(assignment) => self.walk_assignment(assignment),
                    }
                }
                self.walk_redirects(&command.redirects);
            }
            Command::Pipeline(command) => self.walk_commands(&command.commands),
            Command::List(CommandList { first, rest, .. }) => {
                self.walk_command(first);
                for (_, command) in rest {
                    self.walk_command(command);
                }
            }
            Command::Compound(command, redirects) => {
                self.walk_compound(command);
                self.walk_redirects(redirects);
            }
            Command::Function(FunctionDef { body, .. }) => self.walk_command(body),
        }
    }

    fn walk_builtin(&mut self, command: &BuiltinCommand) {
        match command {
            BuiltinCommand::Break(command) => {
                self.walk_assignments(&command.assignments);
                if let Some(word) = &command.depth {
                    self.walk_word(word);
                }
                self.walk_words(&command.extra_args);
                self.walk_redirects(&command.redirects);
            }
            BuiltinCommand::Continue(command) => {
                self.walk_assignments(&command.assignments);
                if let Some(word) = &command.depth {
                    self.walk_word(word);
                }
                self.walk_words(&command.extra_args);
                self.walk_redirects(&command.redirects);
            }
            BuiltinCommand::Return(command) => {
                self.walk_assignments(&command.assignments);
                if let Some(word) = &command.code {
                    self.walk_word(word);
                }
                self.walk_words(&command.extra_args);
                self.walk_redirects(&command.redirects);
            }
            BuiltinCommand::Exit(command) => {
                self.walk_assignments(&command.assignments);
                if let Some(word) = &command.code {
                    self.walk_word(word);
                }
                self.walk_words(&command.extra_args);
                self.walk_redirects(&command.redirects);
            }
        }
    }

    fn walk_compound(&mut self, command: &CompoundCommand) {
        match command {
            CompoundCommand::If(command) => {
                self.walk_commands(&command.condition);
                self.walk_commands(&command.then_branch);
                for (condition, body) in &command.elif_branches {
                    self.walk_commands(condition);
                    self.walk_commands(body);
                }
                if let Some(body) = &command.else_branch {
                    self.walk_commands(body);
                }
            }
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    self.walk_words(words);
                }
                self.walk_commands(&command.body);
            }
            CompoundCommand::ArithmeticFor(command) => self.walk_commands(&command.body),
            CompoundCommand::While(command) => {
                self.walk_commands(&command.condition);
                self.walk_commands(&command.body);
            }
            CompoundCommand::Until(command) => {
                self.walk_commands(&command.condition);
                self.walk_commands(&command.body);
            }
            CompoundCommand::Case(command) => {
                self.walk_word(&command.word);
                for case in &command.cases {
                    self.walk_words(&case.patterns);
                    self.walk_commands(&case.commands);
                }
            }
            CompoundCommand::Select(command) => {
                self.walk_words(&command.words);
                self.walk_commands(&command.body);
            }
            CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
                self.walk_commands(commands);
            }
            CompoundCommand::Arithmetic(_) => {}
            CompoundCommand::Time(command) => {
                if let Some(command) = &command.command {
                    self.walk_command(command);
                }
            }
            CompoundCommand::Conditional(command) => {
                self.walk_conditional_expr(&command.expression);
            }
            CompoundCommand::Coproc(command) => self.walk_command(&command.body),
        }
    }

    fn walk_assignments(&mut self, assignments: &[Assignment]) {
        for assignment in assignments {
            self.walk_assignment(assignment);
        }
    }

    fn walk_assignment(&mut self, assignment: &Assignment) {
        match &assignment.value {
            AssignmentValue::Scalar(word) => self.walk_word(word),
            AssignmentValue::Array(words) => self.walk_words(words),
        }
    }

    fn walk_words(&mut self, words: &[Word]) {
        for word in words {
            self.walk_word(word);
        }
    }

    fn walk_word(&mut self, word: &Word) {
        (self.visitor)(word);

        for part in &word.parts {
            match part {
                WordPart::CommandSubstitution(commands)
                | WordPart::ProcessSubstitution { commands, .. } => self.walk_commands(commands),
                _ => {}
            }
        }
    }

    fn walk_redirects(&mut self, redirects: &[Redirect]) {
        for redirect in redirects {
            self.walk_word(&redirect.target);
        }
    }

    fn walk_conditional_expr(&mut self, expression: &ConditionalExpr) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                self.walk_conditional_expr(&expr.left);
                self.walk_conditional_expr(&expr.right);
            }
            ConditionalExpr::Unary(expr) => self.walk_conditional_expr(&expr.expr),
            ConditionalExpr::Parenthesized(expr) => self.walk_conditional_expr(&expr.expr),
            ConditionalExpr::Word(word)
            | ConditionalExpr::Pattern(word)
            | ConditionalExpr::Regex(word) => self.walk_word(word),
        }
    }
}
