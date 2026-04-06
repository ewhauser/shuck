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

pub fn simple_command_name(command: &SimpleCommand, source: &str) -> Option<String> {
    static_word_text(&command.name, source)
}

pub fn effective_command_name(command: &Command, source: &str) -> Option<String> {
    let Command::Simple(command) = command else {
        return None;
    };

    let name = simple_command_name(command, source)?;
    let effective = match name.as_str() {
        "command" => command_wrapper_target(command, source),
        "exec" => exec_wrapper_target(command, source),
        "busybox" => first_static_arg(command, source),
        "find" => find_exec_target(command, source),
        "git" => git_subcommand_name(command, source),
        "mumps" => mumps_subcommand_name(command, source),
        _ => None,
    };

    Some(effective.unwrap_or(name))
}

pub fn assignment_target_name(assignment: &Assignment) -> &str {
    assignment.name.as_str()
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
        Command::Simple(command) => simple_command_name(command, source).as_deref() == Some(name),
        _ => false,
    }
}

pub fn simple_test_operands<'a>(command: &'a SimpleCommand, source: &str) -> Option<&'a [Word]> {
    let name = simple_command_name(command, source)?;
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

fn first_static_arg(command: &SimpleCommand, source: &str) -> Option<String> {
    command
        .args
        .first()
        .and_then(|arg| static_word_text(arg, source))
}

fn command_wrapper_target(command: &SimpleCommand, source: &str) -> Option<String> {
    let mut index = 0usize;

    while index < command.args.len() {
        let arg = static_word_text(&command.args[index], source)?;
        match arg.as_str() {
            "--" => {
                return command
                    .args
                    .get(index + 1)
                    .and_then(|arg| static_word_text(arg, source));
            }
            "-p" => index += 1,
            "-v" | "-V" => return None,
            _ if arg.starts_with('-') => return None,
            _ => return Some(arg),
        }
    }

    None
}

fn exec_wrapper_target(command: &SimpleCommand, source: &str) -> Option<String> {
    let mut index = 0usize;

    while index < command.args.len() {
        let arg = static_word_text(&command.args[index], source)?;
        match arg.as_str() {
            "--" => {
                return command
                    .args
                    .get(index + 1)
                    .and_then(|arg| static_word_text(arg, source));
            }
            "-c" | "-l" => index += 1,
            "-a" => {
                static_word_text(command.args.get(index + 1)?, source)?;
                index += 2;
            }
            _ if arg.starts_with('-') => return None,
            _ => return Some(arg),
        }
    }

    None
}

fn find_exec_target(command: &SimpleCommand, source: &str) -> Option<String> {
    for (index, arg) in command.args.iter().enumerate() {
        let arg = static_word_text(arg, source)?;
        if matches!(arg.as_str(), "-exec" | "-execdir" | "-ok" | "-okdir") {
            return command
                .args
                .get(index + 1)
                .and_then(|arg| static_word_text(arg, source));
        }
    }

    None
}

fn git_subcommand_name(command: &SimpleCommand, source: &str) -> Option<String> {
    (command
        .args
        .first()
        .and_then(|arg| static_word_text(arg, source))
        .as_deref()
        == Some("filter-branch"))
    .then(|| "git filter-branch".to_owned())
}

fn mumps_subcommand_name(command: &SimpleCommand, source: &str) -> Option<String> {
    let run_flag = command
        .args
        .first()
        .and_then(|arg| static_word_text(arg, source))?;
    let entrypoint = command
        .args
        .get(1)
        .and_then(|arg| static_word_text(arg, source))?;
    if run_flag == "-run" && matches!(entrypoint.as_str(), "%XCMD" | "LOOP%XCMD") {
        Some(format!("mumps -run {entrypoint}"))
    } else {
        None
    }
}

pub fn visit_command_words(command: &Command, visitor: &mut impl FnMut(&Word)) {
    match command {
        Command::Simple(command) => {
            for assignment in &command.assignments {
                match &assignment.value {
                    AssignmentValue::Scalar(word) => visitor(word),
                    AssignmentValue::Array(words) => {
                        for word in words {
                            visitor(word);
                        }
                    }
                }
            }
            visitor(&command.name);
            for word in &command.args {
                visitor(word);
            }
            for redirect in &command.redirects {
                visitor(&redirect.target);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                for assignment in &command.assignments {
                    match &assignment.value {
                        AssignmentValue::Scalar(word) => visitor(word),
                        AssignmentValue::Array(words) => {
                            for word in words {
                                visitor(word);
                            }
                        }
                    }
                }
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
                for redirect in &command.redirects {
                    visitor(&redirect.target);
                }
            }
            BuiltinCommand::Continue(command) => {
                for assignment in &command.assignments {
                    match &assignment.value {
                        AssignmentValue::Scalar(word) => visitor(word),
                        AssignmentValue::Array(words) => {
                            for word in words {
                                visitor(word);
                            }
                        }
                    }
                }
                if let Some(word) = &command.depth {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
                for redirect in &command.redirects {
                    visitor(&redirect.target);
                }
            }
            BuiltinCommand::Return(command) => {
                for assignment in &command.assignments {
                    match &assignment.value {
                        AssignmentValue::Scalar(word) => visitor(word),
                        AssignmentValue::Array(words) => {
                            for word in words {
                                visitor(word);
                            }
                        }
                    }
                }
                if let Some(word) = &command.code {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
                for redirect in &command.redirects {
                    visitor(&redirect.target);
                }
            }
            BuiltinCommand::Exit(command) => {
                for assignment in &command.assignments {
                    match &assignment.value {
                        AssignmentValue::Scalar(word) => visitor(word),
                        AssignmentValue::Array(words) => {
                            for word in words {
                                visitor(word);
                            }
                        }
                    }
                }
                if let Some(word) = &command.code {
                    visitor(word);
                }
                for word in &command.extra_args {
                    visitor(word);
                }
                for redirect in &command.redirects {
                    visitor(&redirect.target);
                }
            }
        },
        Command::Decl(command) => {
            for assignment in &command.assignments {
                match &assignment.value {
                    AssignmentValue::Scalar(word) => visitor(word),
                    AssignmentValue::Array(words) => {
                        for word in words {
                            visitor(word);
                        }
                    }
                }
            }
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => visitor(word),
                    DeclOperand::Name(_) => {}
                    DeclOperand::Assignment(assignment) => match &assignment.value {
                        AssignmentValue::Scalar(word) => visitor(word),
                        AssignmentValue::Array(words) => {
                            for word in words {
                                visitor(word);
                            }
                        }
                    },
                }
            }
            for redirect in &command.redirects {
                visitor(&redirect.target);
            }
        }
        Command::Pipeline(_) | Command::List(_) | Command::Function(_) => {}
        Command::Compound(command, redirects) => {
            match command {
                CompoundCommand::For(command) => {
                    if let Some(words) = &command.words {
                        for word in words {
                            visitor(word);
                        }
                    }
                }
                CompoundCommand::Case(command) => {
                    visitor(&command.word);
                    for case in &command.cases {
                        for pattern in &case.patterns {
                            visitor(pattern);
                        }
                    }
                }
                CompoundCommand::Select(command) => {
                    for word in &command.words {
                        visitor(word);
                    }
                }
                CompoundCommand::Conditional(command) => {
                    visit_conditional_words(&command.expression, visitor);
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

            for redirect in redirects {
                visitor(&redirect.target);
            }
        }
    }
}

pub fn visit_command_redirects(command: &Command, visitor: &mut impl FnMut(&Redirect)) {
    match command {
        Command::Simple(command) => {
            for redirect in &command.redirects {
                visitor(redirect);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                for redirect in &command.redirects {
                    visitor(redirect);
                }
            }
            BuiltinCommand::Continue(command) => {
                for redirect in &command.redirects {
                    visitor(redirect);
                }
            }
            BuiltinCommand::Return(command) => {
                for redirect in &command.redirects {
                    visitor(redirect);
                }
            }
            BuiltinCommand::Exit(command) => {
                for redirect in &command.redirects {
                    visitor(redirect);
                }
            }
        },
        Command::Decl(command) => {
            for redirect in &command.redirects {
                visitor(redirect);
            }
        }
        Command::Compound(_, redirects) => {
            for redirect in redirects {
                visitor(redirect);
            }
        }
        Command::Pipeline(_) | Command::List(_) | Command::Function(_) => {}
    }
}

fn visit_conditional_words(expression: &ConditionalExpr, visitor: &mut impl FnMut(&Word)) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            visit_conditional_words(&expr.left, visitor);
            visit_conditional_words(&expr.right, visitor);
        }
        ConditionalExpr::Unary(expr) => visit_conditional_words(&expr.expr, visitor),
        ConditionalExpr::Parenthesized(expr) => visit_conditional_words(&expr.expr, visitor),
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => visitor(word),
    }
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

#[cfg(test)]
mod tests {
    use shuck_ast::{Command, DeclOperand};
    use shuck_parser::parser::Parser;

    use super::{assignment_target_name, effective_command_name, simple_command_name};

    fn parse_first_command(source: &str) -> Command {
        let output = Parser::new(source).parse().unwrap();
        output.script.commands.into_iter().next().unwrap()
    }

    #[test]
    fn simple_command_name_returns_static_command_name() {
        let source = "printf '%s\\n' hello\n";
        let command = parse_first_command(source);
        let Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(
            simple_command_name(&command, source).as_deref(),
            Some("printf")
        );
    }

    #[test]
    fn simple_command_name_returns_none_for_dynamic_command_name() {
        let source = "\"$tool\" --help\n";
        let command = parse_first_command(source);
        let Command::Simple(command) = command else {
            panic!("expected simple command");
        };

        assert_eq!(simple_command_name(&command, source), None);
    }

    #[test]
    fn effective_command_name_unwraps_known_wrappers() {
        let cases = [
            ("command jq '$__loc__'\n", Some("jq")),
            ("exec jq '$__loc__'\n", Some("jq")),
            ("exec -c -a foo jq '$__loc__'\n", Some("jq")),
            ("busybox awk '{print $1}'\n", Some("awk")),
            ("find . -exec awk '{print $1}' {} \\;\n", Some("awk")),
            (
                "git filter-branch 'test $GIT_COMMIT'\n",
                Some("git filter-branch"),
            ),
            (
                "mumps -run %XCMD 'W $O(^GLOBAL(5))'\n",
                Some("mumps -run %XCMD"),
            ),
            ("printf '%s\\n' hello\n", Some("printf")),
        ];

        for (source, expected) in cases {
            let command = parse_first_command(source);
            assert_eq!(
                effective_command_name(&command, source).as_deref(),
                expected
            );
        }
    }

    #[test]
    fn effective_command_name_falls_back_when_wrapper_target_is_not_static() {
        let cases = [
            ("command \"$tool\" '$__loc__'\n", Some("command")),
            ("exec \"$tool\" '$__loc__'\n", Some("exec")),
            ("find . -exec \"$tool\" {} \\;\n", Some("find")),
            ("git \"$subcommand\" 'test $GIT_COMMIT'\n", Some("git")),
        ];

        for (source, expected) in cases {
            let command = parse_first_command(source);
            assert_eq!(
                effective_command_name(&command, source).as_deref(),
                expected
            );
        }
    }

    #[test]
    fn assignment_target_name_returns_assignment_name() {
        let source = "export PS1='$PWD'\n";
        let command = parse_first_command(source);
        let Command::Decl(command) = command else {
            panic!("expected declaration command");
        };
        let DeclOperand::Assignment(assignment) = &command.operands[0] else {
            panic!("expected declaration assignment");
        };

        assert_eq!(assignment_target_name(assignment), "PS1");
    }
}
