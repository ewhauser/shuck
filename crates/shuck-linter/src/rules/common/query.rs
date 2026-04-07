use shuck_ast::{
    ArrayElem, Assignment, AssignmentValue, BuiltinCommand, Command, CommandList,
    CompoundCommand, ConditionalExpr, DeclOperand, FunctionDef, Pattern, PatternPart, Redirect,
    RedirectKind, Span, Word, WordPart, WordPartNode,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WalkContext {
    pub loop_depth: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CommandWalkOptions {
    pub descend_nested_word_commands: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CommandVisit<'a> {
    pub command: &'a Command,
    pub context: WalkContext,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSubstitutionKind {
    Command,
    ProcessInput,
    ProcessOutput,
}

#[derive(Debug, Clone, Copy)]
pub struct NestedCommandSubstitution<'a> {
    pub commands: &'a [Command],
    pub span: Span,
    pub kind: CommandSubstitutionKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExpansionWordKind {
    CommandArgument,
    RedirectTarget(RedirectKind),
    ForList,
    SelectList,
}

pub fn walk_commands(
    commands: &[Command],
    options: CommandWalkOptions,
    visitor: &mut impl FnMut(&Command, WalkContext),
) {
    CommandWalker { options, visitor }.walk_commands(commands, WalkContext::default());
}

pub fn iter_commands<'a>(
    commands: &'a [Command],
    options: CommandWalkOptions,
) -> impl Iterator<Item = CommandVisit<'a>> {
    let mut visits = Vec::new();
    collect_command_visits(commands, options, WalkContext::default(), &mut visits);
    visits.into_iter()
}

pub fn walk_words(
    commands: &[Command],
    options: CommandWalkOptions,
    visitor: &mut impl FnMut(&Word),
) {
    WordWalker { options, visitor }.walk_commands(commands);
}

pub fn command_assignments(command: &Command) -> &[Assignment] {
    match command {
        Command::Simple(command) => &command.assignments,
        Command::Builtin(command) => builtin_assignments(command),
        Command::Decl(command) => &command.assignments,
        Command::Pipeline(_)
        | Command::List(_)
        | Command::Compound(_, _)
        | Command::Function(_) => &[],
    }
}

pub fn declaration_operands(command: &Command) -> &[DeclOperand] {
    match command {
        Command::Decl(command) => &command.operands,
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Pipeline(_)
        | Command::List(_)
        | Command::Compound(_, _)
        | Command::Function(_) => &[],
    }
}

pub fn command_redirects(command: &Command) -> &[Redirect] {
    match command {
        Command::Simple(command) => &command.redirects,
        Command::Builtin(command) => builtin_redirects(command),
        Command::Decl(command) => &command.redirects,
        Command::Compound(_, redirects) => redirects,
        Command::Pipeline(_) | Command::List(_) | Command::Function(_) => &[],
    }
}

pub fn iter_command_words(command: &Command) -> impl Iterator<Item = &Word> {
    let mut words = Vec::new();
    collect_command_words(command, &mut words);
    words.into_iter()
}

pub fn iter_word_command_substitutions<'a>(
    word: &'a Word,
) -> impl Iterator<Item = NestedCommandSubstitution<'a>> + 'a {
    let mut substitutions = Vec::new();
    collect_word_command_substitutions(&word.parts, &mut substitutions);
    substitutions.into_iter()
}

pub fn iter_command_substitutions<'a>(
    command: &'a Command,
) -> impl Iterator<Item = NestedCommandSubstitution<'a>> {
    let mut substitutions = Vec::new();
    for word in iter_command_words(command) {
        substitutions.extend(iter_word_command_substitutions(word));
    }
    substitutions.into_iter()
}

pub fn visit_command_words(command: &Command, visitor: &mut impl FnMut(&Word)) {
    for word in iter_command_words(command) {
        visitor(word);
    }
}

pub fn visit_argument_words(command: &Command, visitor: &mut impl FnMut(&Word)) {
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

pub fn visit_expansion_words(
    command: &Command,
    visitor: &mut impl FnMut(&Word, ExpansionWordKind),
) {
    visit_argument_words(command, &mut |word| {
        visitor(word, ExpansionWordKind::CommandArgument)
    });

    match command {
        Command::Compound(command, _) => match command {
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    for word in words {
                        visitor(word, ExpansionWordKind::ForList);
                    }
                }
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    visitor(word, ExpansionWordKind::SelectList);
                }
            }
            CompoundCommand::If(_)
            | CompoundCommand::ArithmeticFor(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Case(_)
            | CompoundCommand::Subshell(_)
            | CompoundCommand::BraceGroup(_)
            | CompoundCommand::Arithmetic(_)
            | CompoundCommand::Conditional(_)
            | CompoundCommand::Coproc(_)
            | CompoundCommand::Time(_) => {}
        },
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Decl(_)
        | Command::Pipeline(_)
        | Command::List(_)
        | Command::Function(_) => {}
    }

    for redirect in command_redirects(command) {
        if matches!(
            redirect.kind,
            RedirectKind::HereDoc | RedirectKind::HereDocStrip
        ) {
            continue;
        }
        visitor(
            redirect
                .word_target()
                .expect("expected non-heredoc redirect target"),
            ExpansionWordKind::RedirectTarget(redirect.kind),
        );
    }
}

pub fn visit_command_redirects(command: &Command, visitor: &mut impl FnMut(&Redirect)) {
    for redirect in command_redirects(command) {
        visitor(redirect);
    }
}

fn collect_command_visits<'a>(
    commands: &'a [Command],
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for command in commands {
        collect_command_visit(command, options, context, visits);
    }
}

fn collect_command_visit<'a>(
    command: &'a Command,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    visits.push(CommandVisit { command, context });

    match command {
        Command::Simple(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            collect_word_visits(&command.name, options, context, visits);
            collect_word_slice_visits(&command.args, options, context, visits);
            collect_redirect_visits(&command.redirects, options, context, visits);
        }
        Command::Builtin(command) => collect_builtin_visits(command, options, context, visits),
        Command::Decl(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_word_visits(word, options, context, visits);
                    }
                    DeclOperand::Name(_) => {}
                    DeclOperand::Assignment(assignment) => {
                        collect_assignment_visit(assignment, options, context, visits);
                    }
                }
            }
            collect_redirect_visits(&command.redirects, options, context, visits);
        }
        Command::Pipeline(command) => {
            collect_command_visits(&command.commands, options, context, visits);
        }
        Command::List(CommandList { first, rest, .. }) => {
            collect_command_visit(first, options, context, visits);
            for item in rest {
                collect_command_visit(&item.command, options, context, visits);
            }
        }
        Command::Compound(command, redirects) => {
            collect_compound_visits(command, options, context, visits);
            collect_redirect_visits(redirects, options, context, visits);
        }
        Command::Function(FunctionDef { body, .. }) => {
            collect_command_visit(body, options, context, visits);
        }
    }
}

fn collect_builtin_visits<'a>(
    command: &'a BuiltinCommand,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    match command {
        BuiltinCommand::Break(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            if let Some(word) = &command.depth {
                collect_word_visits(word, options, context, visits);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visits);
            collect_redirect_visits(&command.redirects, options, context, visits);
        }
        BuiltinCommand::Continue(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            if let Some(word) = &command.depth {
                collect_word_visits(word, options, context, visits);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visits);
            collect_redirect_visits(&command.redirects, options, context, visits);
        }
        BuiltinCommand::Return(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            if let Some(word) = &command.code {
                collect_word_visits(word, options, context, visits);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visits);
            collect_redirect_visits(&command.redirects, options, context, visits);
        }
        BuiltinCommand::Exit(command) => {
            collect_assignment_visits(&command.assignments, options, context, visits);
            if let Some(word) = &command.code {
                collect_word_visits(word, options, context, visits);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visits);
            collect_redirect_visits(&command.redirects, options, context, visits);
        }
    }
}

fn collect_compound_visits<'a>(
    command: &'a CompoundCommand,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    match command {
        CompoundCommand::If(command) => {
            collect_command_visits(&command.condition, options, context, visits);
            collect_command_visits(&command.then_branch, options, context, visits);
            for (condition, body) in &command.elif_branches {
                collect_command_visits(condition, options, context, visits);
                collect_command_visits(body, options, context, visits);
            }
            if let Some(body) = &command.else_branch {
                collect_command_visits(body, options, context, visits);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                collect_word_slice_visits(words, options, context, visits);
            }
            collect_command_visits(
                &command.body,
                options,
                WalkContext {
                    loop_depth: context.loop_depth + 1,
                },
                visits,
            );
        }
        CompoundCommand::ArithmeticFor(command) => collect_command_visits(
            &command.body,
            options,
            WalkContext {
                loop_depth: context.loop_depth + 1,
            },
            visits,
        ),
        CompoundCommand::While(command) => {
            let loop_context = WalkContext {
                loop_depth: context.loop_depth + 1,
            };
            collect_command_visits(&command.condition, options, loop_context, visits);
            collect_command_visits(&command.body, options, loop_context, visits);
        }
        CompoundCommand::Until(command) => {
            let loop_context = WalkContext {
                loop_depth: context.loop_depth + 1,
            };
            collect_command_visits(&command.condition, options, loop_context, visits);
            collect_command_visits(&command.body, options, loop_context, visits);
        }
        CompoundCommand::Case(command) => {
            collect_word_visits(&command.word, options, context, visits);
            for case in &command.cases {
                collect_pattern_slice_visits(&case.patterns, options, context, visits);
                collect_command_visits(&case.commands, options, context, visits);
            }
        }
        CompoundCommand::Select(command) => {
            collect_word_slice_visits(&command.words, options, context, visits);
            collect_command_visits(
                &command.body,
                options,
                WalkContext {
                    loop_depth: context.loop_depth + 1,
                },
                visits,
            );
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            collect_command_visits(commands, options, context, visits);
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_command_visit(command, options, context, visits);
            }
        }
        CompoundCommand::Conditional(command) => {
            collect_conditional_visits(&command.expression, options, context, visits);
        }
        CompoundCommand::Coproc(command) => {
            collect_command_visit(&command.body, options, context, visits);
        }
    }
}

fn collect_assignment_visits<'a>(
    assignments: &'a [Assignment],
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for assignment in assignments {
        collect_assignment_visit(assignment, options, context, visits);
    }
}

fn collect_assignment_visit<'a>(
    assignment: &'a Assignment,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_word_visits(word, options, context, visits),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => {
                        collect_word_visits(word, options, context, visits);
                    }
                    ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                        collect_word_visits(value, options, context, visits);
                    }
                }
            }
        }
    }
}

fn collect_word_slice_visits<'a>(
    words: &'a [Word],
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for word in words {
        collect_word_visits(word, options, context, visits);
    }
}

fn collect_pattern_slice_visits<'a>(
    patterns: &'a [Pattern],
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for pattern in patterns {
        collect_pattern_visits(pattern, options, context, visits);
    }
}

fn collect_word_visits<'a>(
    word: &'a Word,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    if !options.descend_nested_word_commands {
        return;
    }

    for substitution in iter_word_command_substitutions(word) {
        collect_command_visits(substitution.commands, options, context, visits);
    }
}

fn collect_pattern_visits<'a>(
    pattern: &'a Pattern,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                collect_pattern_slice_visits(patterns, options, context, visits);
            }
            PatternPart::Word(word) => collect_word_visits(word, options, context, visits),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn collect_redirect_visits<'a>(
    redirects: &'a [Redirect],
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    for redirect in redirects {
        collect_word_visits(redirect_walk_word(redirect), options, context, visits);
    }
}

fn collect_conditional_visits<'a>(
    expression: &'a ConditionalExpr,
    options: CommandWalkOptions,
    context: WalkContext,
    visits: &mut Vec<CommandVisit<'a>>,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_visits(&expr.left, options, context, visits);
            collect_conditional_visits(&expr.right, options, context, visits);
        }
        ConditionalExpr::Unary(expr) => {
            collect_conditional_visits(&expr.expr, options, context, visits)
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_visits(&expr.expr, options, context, visits);
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            collect_word_visits(word, options, context, visits)
        }
        ConditionalExpr::Pattern(pattern) => {
            collect_pattern_visits(pattern, options, context, visits)
        }
        ConditionalExpr::VarRef(_) => {}
    }
}

struct CommandWalker<'a, F> {
    options: CommandWalkOptions,
    visitor: &'a mut F,
}

impl<F: FnMut(&Command, WalkContext)> CommandWalker<'_, F> {
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
                for item in rest {
                    self.walk_command(&item.command, context);
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
                    self.walk_patterns(&case.patterns, context);
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
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word) => self.walk_word(word, context),
                        ArrayElem::Keyed { value, .. }
                        | ArrayElem::KeyedAppend { value, .. } => self.walk_word(value, context),
                    }
                }
            }
        }
    }

    fn walk_words(&mut self, words: &[Word], context: WalkContext) {
        for word in words {
            self.walk_word(word, context);
        }
    }

    fn walk_patterns(&mut self, patterns: &[Pattern], context: WalkContext) {
        for pattern in patterns {
            self.walk_pattern(pattern, context);
        }
    }

    fn walk_word(&mut self, word: &Word, context: WalkContext) {
        if !self.options.descend_nested_word_commands {
            return;
        }

        self.walk_word_parts(&word.parts, context);
    }

    fn walk_pattern(&mut self, pattern: &Pattern, context: WalkContext) {
        for (part, _) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { patterns, .. } => self.walk_patterns(patterns, context),
                PatternPart::Word(word) => self.walk_word(word, context),
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_) => {}
            }
        }
    }

    fn walk_word_parts(&mut self, parts: &[WordPartNode], context: WalkContext) {
        for part in parts {
            match &part.kind {
                WordPart::DoubleQuoted { parts, .. } => self.walk_word_parts(parts, context),
                WordPart::CommandSubstitution { commands, .. }
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
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                self.walk_word(word, context)
            }
            ConditionalExpr::Pattern(pattern) => self.walk_pattern(pattern, context),
            ConditionalExpr::VarRef(_) => {}
        }
    }

    fn walk_redirects(&mut self, redirects: &[Redirect], context: WalkContext) {
        for redirect in redirects {
            self.walk_word(redirect_walk_word(redirect), context);
        }
    }
}

struct WordWalker<'a, F> {
    options: CommandWalkOptions,
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
                            self.walk_word(word);
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
                for item in rest {
                    self.walk_command(&item.command);
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
                    self.walk_patterns(&case.patterns);
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
            AssignmentValue::Compound(array) => {
                for element in &array.elements {
                    match element {
                        ArrayElem::Sequential(word) => self.walk_word(word),
                        ArrayElem::Keyed { value, .. }
                        | ArrayElem::KeyedAppend { value, .. } => self.walk_word(value),
                    }
                }
            }
        }
    }

    fn walk_words(&mut self, words: &[Word]) {
        for word in words {
            self.walk_word(word);
        }
    }

    fn walk_patterns(&mut self, patterns: &[Pattern]) {
        for pattern in patterns {
            self.walk_pattern(pattern);
        }
    }

    fn walk_word(&mut self, word: &Word) {
        (self.visitor)(word);

        if !self.options.descend_nested_word_commands {
            return;
        }

        self.walk_word_parts(&word.parts);
    }

    fn walk_pattern(&mut self, pattern: &Pattern) {
        for (part, _) in pattern.parts_with_spans() {
            match part {
                PatternPart::Group { patterns, .. } => self.walk_patterns(patterns),
                PatternPart::Word(word) => self.walk_word(word),
                PatternPart::Literal(_)
                | PatternPart::AnyString
                | PatternPart::AnyChar
                | PatternPart::CharClass(_) => {}
            }
        }
    }

    fn walk_word_parts(&mut self, parts: &[WordPartNode]) {
        for part in parts {
            match &part.kind {
                WordPart::DoubleQuoted { parts, .. } => self.walk_word_parts(parts),
                WordPart::CommandSubstitution { commands, .. }
                | WordPart::ProcessSubstitution { commands, .. } => self.walk_commands(commands),
                _ => {}
            }
        }
    }

    fn walk_redirects(&mut self, redirects: &[Redirect]) {
        for redirect in redirects {
            self.walk_word(redirect_walk_word(redirect));
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
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => self.walk_word(word),
            ConditionalExpr::Pattern(pattern) => self.walk_pattern(pattern),
            ConditionalExpr::VarRef(_) => {}
        }
    }
}

fn builtin_assignments(command: &BuiltinCommand) -> &[Assignment] {
    match command {
        BuiltinCommand::Break(command) => &command.assignments,
        BuiltinCommand::Continue(command) => &command.assignments,
        BuiltinCommand::Return(command) => &command.assignments,
        BuiltinCommand::Exit(command) => &command.assignments,
    }
}

fn builtin_redirects(command: &BuiltinCommand) -> &[Redirect] {
    match command {
        BuiltinCommand::Break(command) => &command.redirects,
        BuiltinCommand::Continue(command) => &command.redirects,
        BuiltinCommand::Return(command) => &command.redirects,
        BuiltinCommand::Exit(command) => &command.redirects,
    }
}

fn collect_command_words<'a>(command: &'a Command, words: &mut Vec<&'a Word>) {
    match command {
        Command::Simple(command) => {
            collect_assignments_words(&command.assignments, words);
            words.push(&command.name);
            collect_words(&command.args, words);
            collect_redirect_target_words(&command.redirects, words);
        }
        Command::Builtin(command) => collect_builtin_words(command, words),
        Command::Decl(command) => {
            collect_assignments_words(&command.assignments, words);
            for operand in &command.operands {
                collect_decl_operand_words(operand, words);
            }
            collect_redirect_target_words(&command.redirects, words);
        }
        Command::Pipeline(_) | Command::List(_) | Command::Function(_) => {}
        Command::Compound(command, redirects) => {
            match command {
                CompoundCommand::For(command) => {
                    if let Some(command_words) = &command.words {
                        collect_words(command_words, words);
                    }
                }
                CompoundCommand::Case(command) => {
                    words.push(&command.word);
                    for case in &command.cases {
                        collect_pattern_words(&case.patterns, words);
                    }
                }
                CompoundCommand::Select(command) => collect_words(&command.words, words),
                CompoundCommand::Conditional(command) => {
                    collect_conditional_words(&command.expression, words);
                }
                CompoundCommand::If(_)
                | CompoundCommand::ArithmeticFor(_)
                | CompoundCommand::While(_)
                | CompoundCommand::Until(_)
                | CompoundCommand::Subshell(_)
                | CompoundCommand::BraceGroup(_)
                | CompoundCommand::Arithmetic(_)
                | CompoundCommand::Time(_)
                | CompoundCommand::Coproc(_) => {}
            }

            collect_redirect_target_words(redirects, words);
        }
    }
}

fn collect_assignments_words<'a>(assignments: &'a [Assignment], words: &mut Vec<&'a Word>) {
    for assignment in assignments {
        collect_assignment_words(assignment, words);
    }
}

fn collect_assignment_words<'a>(assignment: &'a Assignment, words: &mut Vec<&'a Word>) {
    match &assignment.value {
        AssignmentValue::Scalar(word) => words.push(word),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => words.push(word),
                    ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                        words.push(value);
                    }
                }
            }
        }
    }
}

fn collect_builtin_words<'a>(command: &'a BuiltinCommand, words: &mut Vec<&'a Word>) {
    match command {
        BuiltinCommand::Break(command) => {
            collect_assignments_words(&command.assignments, words);
            if let Some(word) = &command.depth {
                words.push(word);
            }
            collect_words(&command.extra_args, words);
            collect_redirect_target_words(&command.redirects, words);
        }
        BuiltinCommand::Continue(command) => {
            collect_assignments_words(&command.assignments, words);
            if let Some(word) = &command.depth {
                words.push(word);
            }
            collect_words(&command.extra_args, words);
            collect_redirect_target_words(&command.redirects, words);
        }
        BuiltinCommand::Return(command) => {
            collect_assignments_words(&command.assignments, words);
            if let Some(word) = &command.code {
                words.push(word);
            }
            collect_words(&command.extra_args, words);
            collect_redirect_target_words(&command.redirects, words);
        }
        BuiltinCommand::Exit(command) => {
            collect_assignments_words(&command.assignments, words);
            if let Some(word) = &command.code {
                words.push(word);
            }
            collect_words(&command.extra_args, words);
            collect_redirect_target_words(&command.redirects, words);
        }
    }
}

fn collect_word_command_substitutions<'a>(
    parts: &'a [WordPartNode],
    substitutions: &mut Vec<NestedCommandSubstitution<'a>>,
) {
    for part in parts {
        match &part.kind {
            WordPart::DoubleQuoted { parts, .. } => {
                collect_word_command_substitutions(parts, substitutions);
            }
            WordPart::CommandSubstitution { commands, .. } => {
                substitutions.push(NestedCommandSubstitution {
                    commands,
                    span: part.span,
                    kind: CommandSubstitutionKind::Command,
                });
            }
            WordPart::ProcessSubstitution { commands, is_input } => {
                substitutions.push(NestedCommandSubstitution {
                    commands,
                    span: part.span,
                    kind: if *is_input {
                        CommandSubstitutionKind::ProcessInput
                    } else {
                        CommandSubstitutionKind::ProcessOutput
                    },
                });
            }
            _ => {}
        }
    }
}

fn collect_decl_operand_words<'a>(operand: &'a DeclOperand, words: &mut Vec<&'a Word>) {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => words.push(word),
        DeclOperand::Name(_) => {}
        DeclOperand::Assignment(assignment) => collect_assignment_words(assignment, words),
    }
}

fn collect_words<'a>(command_words: &'a [Word], words: &mut Vec<&'a Word>) {
    words.extend(command_words);
}

fn collect_pattern_words<'a>(patterns: &'a [Pattern], words: &mut Vec<&'a Word>) {
    for pattern in patterns {
        collect_pattern_words_from_pattern(pattern, words);
    }
}

fn collect_redirect_target_words<'a>(redirects: &'a [Redirect], words: &mut Vec<&'a Word>) {
    for redirect in redirects {
        words.push(redirect_walk_word(redirect));
    }
}

fn redirect_walk_word(redirect: &Redirect) -> &Word {
    match redirect.word_target() {
        Some(word) => word,
        None => &redirect.heredoc().expect("expected heredoc redirect").body,
    }
}

fn collect_conditional_words<'a>(expression: &'a ConditionalExpr, words: &mut Vec<&'a Word>) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_words(&expr.left, words);
            collect_conditional_words(&expr.right, words);
        }
        ConditionalExpr::Unary(expr) => collect_conditional_words(&expr.expr, words),
        ConditionalExpr::Parenthesized(expr) => collect_conditional_words(&expr.expr, words),
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => words.push(word),
        ConditionalExpr::Pattern(pattern) => collect_pattern_words_from_pattern(pattern, words),
        ConditionalExpr::VarRef(_) => {}
    }
}

fn collect_pattern_words_from_pattern<'a>(pattern: &'a Pattern, words: &mut Vec<&'a Word>) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => collect_pattern_words(patterns, words),
            PatternPart::Word(word) => words.push(word),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, RedirectKind, Word, WordPart};
    use shuck_parser::parser::Parser;

    use super::{
        CommandSubstitutionKind, CommandWalkOptions, ExpansionWordKind, command_assignments,
        command_redirects, declaration_operands, iter_command_substitutions, iter_command_words,
        iter_commands, iter_word_command_substitutions, visit_expansion_words,
    };

    fn parse_commands(source: &str) -> Vec<Command> {
        let output = Parser::new(source).parse().unwrap();
        output.script.commands
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

    #[test]
    fn iter_commands_can_ignore_or_follow_nested_word_commands() {
        let source = "echo \"$(printf x)\"\n";
        let commands = parse_commands(source);

        let structural = iter_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: false,
            },
        )
        .filter_map(|visit| {
            let Command::Simple(command) = visit.command else {
                return None;
            };

            static_word_text(&command.name, source)
        })
        .collect::<Vec<_>>();

        let nested = iter_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        )
        .filter_map(|visit| {
            let Command::Simple(command) = visit.command else {
                return None;
            };

            static_word_text(&command.name, source)
        })
        .collect::<Vec<_>>();

        assert_eq!(structural, vec!["echo"]);
        assert_eq!(nested, vec!["echo", "printf"]);
    }

    #[test]
    fn iter_commands_tracks_loop_depth_across_for_while_and_select() {
        let source = "for item in \"$(printf for-word)\"; do\n    printf for-body\n\
done\nwhile printf while-cond; do\n    printf while-body\n\
done\nselect item in \"$(printf select-word)\"; do\n    printf select-body\n\
done\n";
        let commands = parse_commands(source);
        let seen = iter_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        )
        .filter_map(|visit| {
            let Command::Simple(command) = visit.command else {
                return None;
            };
            let name = static_word_text(&command.name, source)?;
            if name != "printf" {
                return None;
            }

            let label = command
                .args
                .first()
                .and_then(|word| static_word_text(word, source))
                .unwrap();
            Some((label, visit.context.loop_depth))
        })
        .collect::<Vec<_>>();

        assert_eq!(
            seen,
            vec![
                ("for-word".to_owned(), 0),
                ("for-body".to_owned(), 1),
                ("while-cond".to_owned(), 1),
                ("while-body".to_owned(), 1),
                ("select-word".to_owned(), 0),
                ("select-body".to_owned(), 1),
            ]
        );
    }

    #[test]
    fn common_iterators_cover_command_shapes() {
        let source = "foo=1 printf bar >simple\nfoo=1 break 2 bar >builtin\n\
foo=1 export foo=1 >decl\nfor item in foo bar; do :; done >compound\n";
        let commands = parse_commands(source);

        let assignment_names = commands
            .iter()
            .map(|command| {
                command_assignments(command)
                    .iter()
                    .map(|assignment| assignment.target.name.as_str().to_owned())
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        let operand_counts = commands
            .iter()
            .map(|command| declaration_operands(command).len())
            .collect::<Vec<_>>();

        let word_lists: Vec<Vec<String>> = commands
            .iter()
            .map(|command| {
                iter_command_words(command)
                    .map(|word| static_word_text(word, source).unwrap())
                    .collect()
            })
            .collect();

        let redirect_lists: Vec<Vec<String>> = commands
            .iter()
            .map(|command| {
                command_redirects(command)
                    .iter()
                    .map(|redirect| {
                        static_word_text(
                            redirect
                                .word_target()
                                .expect("expected non-heredoc redirect target"),
                            source,
                        )
                        .unwrap()
                    })
                    .collect()
            })
            .collect();

        assert_eq!(
            assignment_names,
            vec![
                vec!["foo".to_owned()],
                vec!["foo".to_owned()],
                vec!["foo".to_owned()],
                Vec::<String>::new(),
            ]
        );
        assert_eq!(operand_counts, vec![0, 0, 1, 0]);
        assert_eq!(
            word_lists,
            vec![
                vec![
                    "1".to_owned(),
                    "printf".to_owned(),
                    "bar".to_owned(),
                    "simple".to_owned(),
                ],
                vec![
                    "1".to_owned(),
                    "2".to_owned(),
                    "bar".to_owned(),
                    "builtin".to_owned(),
                ],
                vec!["1".to_owned(), "1".to_owned(), "decl".to_owned()],
                vec!["foo".to_owned(), "bar".to_owned(), "compound".to_owned(),],
            ]
        );
        assert_eq!(
            redirect_lists,
            vec![
                vec!["simple".to_owned()],
                vec!["builtin".to_owned()],
                vec!["decl".to_owned()],
                vec!["compound".to_owned()],
            ]
        );
    }

    #[test]
    fn word_command_substitution_iterator_reports_kind_and_span() {
        let source = "echo \"$(printf cmd)\" <(printf in) >(printf out)\n";
        let commands = parse_commands(source);
        let substitutions = iter_command_substitutions(&commands[0])
            .map(|substitution| {
                let label = substitution
                    .commands
                    .first()
                    .and_then(|command| match command {
                        Command::Simple(command) => command
                            .args
                            .first()
                            .and_then(|word| static_word_text(word, source)),
                        _ => None,
                    })
                    .unwrap();
                (label, substitution.kind, substitution.span)
            })
            .collect::<Vec<_>>();

        assert_eq!(
            substitutions
                .iter()
                .map(|(label, kind, _)| (label.clone(), *kind))
                .collect::<Vec<_>>(),
            vec![
                ("cmd".to_owned(), CommandSubstitutionKind::Command),
                ("in".to_owned(), CommandSubstitutionKind::ProcessInput),
                ("out".to_owned(), CommandSubstitutionKind::ProcessOutput),
            ]
        );
        assert!(
            substitutions
                .iter()
                .all(|(_, _, span)| span.start.offset < span.end.offset)
        );
    }

    #[test]
    fn command_substitution_iterators_reach_assignments_here_strings_conditionals_wrappers_and_compounds()
     {
        let source = "value=\"$(printf assign)\"\n\
command printf \"$(printf wrapped)\"\n\
cat <<< \"$(printf here)\"\n\
if [[ \"$(printf lhs)\" = \"$(printf rhs)\" ]]; then :; fi\n\
for item in \"$(printf loop)\"; do :; done\n";
        let commands = parse_commands(source);

        let substitution_labels = iter_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
        )
        .map(|visit| {
            iter_command_substitutions(visit.command)
                .map(|substitution| {
                    substitution
                        .commands
                        .first()
                        .and_then(|nested_command| match nested_command {
                            Command::Simple(command) => command
                                .args
                                .first()
                                .and_then(|word| static_word_text(word, source)),
                            _ => None,
                        })
                        .unwrap()
                })
                .collect::<Vec<_>>()
        })
        .filter(|labels| !labels.is_empty())
        .collect::<Vec<_>>();

        assert_eq!(
            substitution_labels,
            vec![
                vec!["assign".to_owned()],
                vec!["wrapped".to_owned()],
                vec!["here".to_owned()],
                vec!["lhs".to_owned(), "rhs".to_owned()],
                vec!["loop".to_owned()],
            ]
        );

        let wrapper_word_substitutions = match &commands[1] {
            Command::Simple(command) => iter_word_command_substitutions(&command.args[1])
                .map(|substitution| substitution.kind)
                .collect::<Vec<_>>(),
            _ => panic!("expected simple command"),
        };
        assert_eq!(
            wrapper_word_substitutions,
            vec![CommandSubstitutionKind::Command]
        );
    }

    #[test]
    fn visit_expansion_words_covers_args_loop_lists_and_non_heredoc_redirects() {
        let source = "\
printf '%s\\n' $arg >$out <<EOF
body
EOF
for item in $first \"$second\"; do :; done
select item in $choice; do :; done
cat <<< $here
";
        let commands = parse_commands(source);
        let mut seen = Vec::new();

        for command in &commands {
            visit_expansion_words(command, &mut |word, kind| {
                seen.push((kind, word.span.slice(source).to_owned()));
            });
        }

        assert_eq!(
            seen,
            vec![
                (ExpansionWordKind::CommandArgument, "'%s\\n'".to_owned()),
                (ExpansionWordKind::CommandArgument, "$arg".to_owned()),
                (
                    ExpansionWordKind::RedirectTarget(RedirectKind::Output),
                    "$out".to_owned(),
                ),
                (ExpansionWordKind::ForList, "$first".to_owned()),
                (ExpansionWordKind::ForList, "\"$second\"".to_owned()),
                (ExpansionWordKind::SelectList, "$choice".to_owned()),
                (
                    ExpansionWordKind::RedirectTarget(RedirectKind::HereString),
                    "$here".to_owned(),
                ),
            ]
        );
    }
}
