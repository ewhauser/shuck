use shuck_ast::{
    Assignment, AssignmentValue, BuiltinCommand, Command, CompoundCommand, ConditionalBinaryExpr,
    ConditionalBinaryOp, ConditionalCommand, ConditionalExpr, ConditionalParenExpr,
    ConditionalUnaryExpr, ConditionalUnaryOp, DeclClause, DeclName, DeclOperand, FunctionDef,
    ParameterOp, Redirect, RedirectTarget, Script, SourceText, Word, WordPart, WordPartNode,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimplifyReport {
    applied: Vec<RewriteApplication>,
}

impl SimplifyReport {
    #[must_use]
    pub fn applied(&self) -> &[RewriteApplication] {
        &self.applied
    }

    #[must_use]
    pub fn total_changes(&self) -> usize {
        self.applied.iter().map(|entry| entry.changes).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteApplication {
    pub name: &'static str,
    pub changes: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct SimplifyRewrite {
    pub name: &'static str,
    pub apply: fn(&mut Script, &str) -> usize,
}

pub const REWRITES: &[SimplifyRewrite] = &[
    SimplifyRewrite {
        name: "paren-cleanup",
        apply: rewrite_paren_cleanup,
    },
    SimplifyRewrite {
        name: "arithmetic-vars",
        apply: rewrite_arithmetic_variables,
    },
    SimplifyRewrite {
        name: "conditionals",
        apply: rewrite_conditionals,
    },
    SimplifyRewrite {
        name: "nested-subshells",
        apply: rewrite_nested_subshells,
    },
    SimplifyRewrite {
        name: "quote-tightening",
        apply: rewrite_quote_tightening,
    },
];

pub fn simplify_script(script: &mut Script, source: &str) -> SimplifyReport {
    let mut applied = Vec::new();
    for rewrite in REWRITES {
        let changes = (rewrite.apply)(script, source);
        if changes > 0 {
            applied.push(RewriteApplication {
                name: rewrite.name,
                changes,
            });
        }
    }
    SimplifyReport { applied }
}

fn rewrite_paren_cleanup(script: &mut Script, source: &str) -> usize {
    walk_commands(script, source, &mut |command, source| {
        let mut changes = 0;
        changes += rewrite_command_source_texts(command, source, &mut |text, source| {
            transform_source_text(text, source, strip_single_outer_parens)
        });
        changes
    })
}

fn rewrite_arithmetic_variables(script: &mut Script, source: &str) -> usize {
    walk_commands(script, source, &mut |command, source| {
        rewrite_command_source_texts(command, source, &mut |text, source| {
            transform_source_text(text, source, simplify_arithmetic_variables_text)
        })
    })
}

fn rewrite_conditionals(script: &mut Script, source: &str) -> usize {
    walk_commands(script, source, &mut |command, source| match command {
        Command::Compound(CompoundCommand::Conditional(conditional), _) => {
            simplify_conditional_command(conditional, source)
        }
        _ => 0,
    })
}

fn rewrite_nested_subshells(script: &mut Script, source: &str) -> usize {
    walk_commands(script, source, &mut |command, source| {
        let mut changes = 0;
        if let Command::Compound(CompoundCommand::Subshell(commands), redirects) = command
            && redirects.is_empty()
        {
            while commands.len() == 1 {
                let Some(Command::Compound(CompoundCommand::Subshell(inner), inner_redirects)) =
                    commands.first()
                else {
                    break;
                };
                if !inner_redirects.is_empty() {
                    break;
                }
                *commands = inner.clone();
                changes += 1;
            }
        }

        changes += rewrite_command_words(command, source, &mut |word, _source| {
            let mut word_changes = 0;
            for part in &mut word.parts {
                match &mut part.kind {
                    WordPart::CommandSubstitution { commands, .. }
                    | WordPart::ProcessSubstitution { commands, .. } => {
                        while commands.len() == 1 {
                            let Some(Command::Compound(
                                CompoundCommand::Subshell(inner),
                                redirects,
                            )) = commands.first()
                            else {
                                break;
                            };
                            if !redirects.is_empty() {
                                break;
                            }
                            *commands = inner.clone();
                            word_changes += 1;
                        }
                    }
                    _ => {}
                }
            }
            word_changes
        });
        changes
    })
}

fn rewrite_quote_tightening(script: &mut Script, source: &str) -> usize {
    walk_commands(script, source, &mut |command, source| {
        rewrite_command_words(command, source, &mut |word, source| {
            tighten_literal_quotes(word, source)
        })
    })
}

fn walk_commands(
    script: &mut Script,
    source: &str,
    visitor: &mut impl FnMut(&mut Command, &str) -> usize,
) -> usize {
    script
        .commands
        .iter_mut()
        .map(|command| walk_command(command, source, visitor))
        .sum()
}

fn walk_command(
    command: &mut Command,
    source: &str,
    visitor: &mut impl FnMut(&mut Command, &str) -> usize,
) -> usize {
    let mut changes = visitor(command, source);
    changes += match command {
        Command::Simple(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += walk_assignment(assignment, source, &mut |_| 0, &mut |_| 0);
            }
            count += walk_word(&mut command.name, source, &mut |_| 0);
            for argument in &mut command.args {
                count += walk_word(argument, source, &mut |_| 0);
            }
            for redirect in &mut command.redirects {
                count += walk_redirect(redirect, source, &mut |_| 0);
            }
            count
        }
        Command::Builtin(command) => walk_builtin(command, source),
        Command::Decl(command) => walk_decl_clause(command, source),
        Command::Pipeline(pipeline) => pipeline
            .commands
            .iter_mut()
            .map(|command| walk_command(command, source, visitor))
            .sum(),
        Command::List(list) => {
            let mut count = walk_command(&mut list.first, source, visitor);
            for item in &mut list.rest {
                count += walk_command(&mut item.command, source, visitor);
            }
            count
        }
        Command::Compound(compound, redirects) => {
            let mut count = walk_compound(compound, source, visitor);
            for redirect in redirects {
                count += walk_redirect(redirect, source, &mut |_| 0);
            }
            count
        }
        Command::Function(function) => walk_function(function, source, visitor),
    };
    changes
}

fn walk_builtin(command: &mut BuiltinCommand, source: &str) -> usize {
    match command {
        BuiltinCommand::Break(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += walk_assignment(assignment, source, &mut |_| 0, &mut |_| 0);
            }
            if let Some(depth) = &mut command.depth {
                count += walk_word(depth, source, &mut |_| 0);
            }
            for argument in &mut command.extra_args {
                count += walk_word(argument, source, &mut |_| 0);
            }
            for redirect in &mut command.redirects {
                count += walk_redirect(redirect, source, &mut |_| 0);
            }
            count
        }
        BuiltinCommand::Continue(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += walk_assignment(assignment, source, &mut |_| 0, &mut |_| 0);
            }
            if let Some(depth) = &mut command.depth {
                count += walk_word(depth, source, &mut |_| 0);
            }
            for argument in &mut command.extra_args {
                count += walk_word(argument, source, &mut |_| 0);
            }
            for redirect in &mut command.redirects {
                count += walk_redirect(redirect, source, &mut |_| 0);
            }
            count
        }
        BuiltinCommand::Return(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += walk_assignment(assignment, source, &mut |_| 0, &mut |_| 0);
            }
            if let Some(code) = &mut command.code {
                count += walk_word(code, source, &mut |_| 0);
            }
            for argument in &mut command.extra_args {
                count += walk_word(argument, source, &mut |_| 0);
            }
            for redirect in &mut command.redirects {
                count += walk_redirect(redirect, source, &mut |_| 0);
            }
            count
        }
        BuiltinCommand::Exit(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += walk_assignment(assignment, source, &mut |_| 0, &mut |_| 0);
            }
            if let Some(code) = &mut command.code {
                count += walk_word(code, source, &mut |_| 0);
            }
            for argument in &mut command.extra_args {
                count += walk_word(argument, source, &mut |_| 0);
            }
            for redirect in &mut command.redirects {
                count += walk_redirect(redirect, source, &mut |_| 0);
            }
            count
        }
    }
}

fn walk_decl_clause(command: &mut DeclClause, source: &str) -> usize {
    let mut count = 0;
    for assignment in &mut command.assignments {
        count += walk_assignment(assignment, source, &mut |_| 0, &mut |_| 0);
    }
    for operand in &mut command.operands {
        count += match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                walk_word(word, source, &mut |_| 0)
            }
            DeclOperand::Name(name) => walk_decl_name(name, source, &mut |_| 0),
            DeclOperand::Assignment(assignment) => {
                walk_assignment(assignment, source, &mut |_| 0, &mut |_| 0)
            }
        };
    }
    for redirect in &mut command.redirects {
        count += walk_redirect(redirect, source, &mut |_| 0);
    }
    count
}

fn walk_function(
    function: &mut FunctionDef,
    source: &str,
    visitor: &mut impl FnMut(&mut Command, &str) -> usize,
) -> usize {
    walk_command(function.body.as_mut(), source, visitor)
}

fn walk_compound(
    command: &mut CompoundCommand,
    source: &str,
    visitor: &mut impl FnMut(&mut Command, &str) -> usize,
) -> usize {
    match command {
        CompoundCommand::If(command) => {
            let mut count = 0;
            for condition in &mut command.condition {
                count += walk_command(condition, source, visitor);
            }
            for then_command in &mut command.then_branch {
                count += walk_command(then_command, source, visitor);
            }
            for (condition, body) in &mut command.elif_branches {
                for command in condition {
                    count += walk_command(command, source, visitor);
                }
                for command in body {
                    count += walk_command(command, source, visitor);
                }
            }
            if let Some(body) = &mut command.else_branch {
                for command in body {
                    count += walk_command(command, source, visitor);
                }
            }
            count
        }
        CompoundCommand::For(command) => {
            let mut count = 0;
            if let Some(words) = &mut command.words {
                for word in words {
                    count += walk_word(word, source, &mut |_| 0);
                }
            }
            for body in &mut command.body {
                count += walk_command(body, source, visitor);
            }
            count
        }
        CompoundCommand::ArithmeticFor(command) => command
            .body
            .iter_mut()
            .map(|body| walk_command(body, source, visitor))
            .sum(),
        CompoundCommand::While(command) => {
            let mut count = 0;
            for condition in &mut command.condition {
                count += walk_command(condition, source, visitor);
            }
            for body in &mut command.body {
                count += walk_command(body, source, visitor);
            }
            count
        }
        CompoundCommand::Until(command) => {
            let mut count = 0;
            for condition in &mut command.condition {
                count += walk_command(condition, source, visitor);
            }
            for body in &mut command.body {
                count += walk_command(body, source, visitor);
            }
            count
        }
        CompoundCommand::Case(command) => {
            let mut count = walk_word(&mut command.word, source, &mut |_| 0);
            for item in &mut command.cases {
                for pattern in &mut item.patterns {
                    count += walk_word(pattern, source, &mut |_| 0);
                }
                for body in &mut item.commands {
                    count += walk_command(body, source, visitor);
                }
            }
            count
        }
        CompoundCommand::Select(command) => {
            let mut count = 0;
            for word in &mut command.words {
                count += walk_word(word, source, &mut |_| 0);
            }
            for body in &mut command.body {
                count += walk_command(body, source, visitor);
            }
            count
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .iter_mut()
            .map(|command| walk_command(command, source, visitor))
            .sum(),
        CompoundCommand::Arithmetic(_) => 0,
        CompoundCommand::Time(command) => command
            .command
            .as_mut()
            .map_or(0, |command| walk_command(command, source, visitor)),
        CompoundCommand::Conditional(command) => {
            let mut count = 0;
            count += walk_conditional_words(&mut command.expression, source, &mut |_| 0);
            count
        }
        CompoundCommand::Coproc(command) => walk_command(command.body.as_mut(), source, visitor),
    }
}

fn walk_redirect(
    redirect: &mut Redirect,
    source: &str,
    word_visitor: &mut impl FnMut(&mut Word) -> usize,
) -> usize {
    match &mut redirect.target {
        RedirectTarget::Word(word) => walk_word(word, source, word_visitor),
        RedirectTarget::Heredoc(heredoc) => {
            walk_word(&mut heredoc.delimiter.raw, source, word_visitor)
                + walk_word(&mut heredoc.body, source, word_visitor)
        }
    }
}

fn rewrite_command_words(
    command: &mut Command,
    source: &str,
    visitor: &mut impl FnMut(&mut Word, &str) -> usize,
) -> usize {
    match command {
        Command::Simple(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += rewrite_assignment_words(assignment, source, visitor);
            }
            count += walk_word(&mut command.name, source, &mut |word| visitor(word, source));
            for argument in &mut command.args {
                count += walk_word(argument, source, &mut |word| visitor(word, source));
            }
            for redirect in &mut command.redirects {
                count += rewrite_redirect_words(redirect, source, visitor);
            }
            count
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => rewrite_builtin_like_words(
                &mut command.assignments,
                command.depth.as_mut(),
                &mut command.extra_args,
                &mut command.redirects,
                source,
                visitor,
            ),
            BuiltinCommand::Continue(command) => rewrite_builtin_like_words(
                &mut command.assignments,
                command.depth.as_mut(),
                &mut command.extra_args,
                &mut command.redirects,
                source,
                visitor,
            ),
            BuiltinCommand::Return(command) => rewrite_builtin_like_words(
                &mut command.assignments,
                command.code.as_mut(),
                &mut command.extra_args,
                &mut command.redirects,
                source,
                visitor,
            ),
            BuiltinCommand::Exit(command) => rewrite_builtin_like_words(
                &mut command.assignments,
                command.code.as_mut(),
                &mut command.extra_args,
                &mut command.redirects,
                source,
                visitor,
            ),
        },
        Command::Decl(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += rewrite_assignment_words(assignment, source, visitor);
            }
            for operand in &mut command.operands {
                count += match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        walk_word(word, source, &mut |word| visitor(word, source))
                    }
                    DeclOperand::Name(_) => 0,
                    DeclOperand::Assignment(assignment) => {
                        rewrite_assignment_words(assignment, source, visitor)
                    }
                };
            }
            for redirect in &mut command.redirects {
                count += rewrite_redirect_words(redirect, source, visitor);
            }
            count
        }
        Command::Pipeline(pipeline) => pipeline
            .commands
            .iter_mut()
            .map(|command| rewrite_command_words(command, source, visitor))
            .sum(),
        Command::List(list) => {
            rewrite_command_words(&mut list.first, source, visitor)
                + list
                    .rest
                    .iter_mut()
                    .map(|item| rewrite_command_words(&mut item.command, source, visitor))
                    .sum::<usize>()
        }
        Command::Compound(compound, redirects) => {
            let mut count = rewrite_compound_words(compound, source, visitor);
            for redirect in redirects {
                count += rewrite_redirect_words(redirect, source, visitor);
            }
            count
        }
        Command::Function(function) => {
            rewrite_command_words(function.body.as_mut(), source, visitor)
        }
    }
}

fn rewrite_builtin_like_words(
    assignments: &mut [Assignment],
    primary: Option<&mut Word>,
    extra_args: &mut [Word],
    redirects: &mut [Redirect],
    source: &str,
    visitor: &mut impl FnMut(&mut Word, &str) -> usize,
) -> usize {
    let mut count = 0;
    for assignment in assignments {
        count += rewrite_assignment_words(assignment, source, visitor);
    }
    if let Some(primary) = primary {
        count += walk_word(primary, source, &mut |word| visitor(word, source));
    }
    for argument in extra_args {
        count += walk_word(argument, source, &mut |word| visitor(word, source));
    }
    for redirect in redirects {
        count += rewrite_redirect_words(redirect, source, visitor);
    }
    count
}

fn rewrite_compound_words(
    command: &mut CompoundCommand,
    source: &str,
    visitor: &mut impl FnMut(&mut Word, &str) -> usize,
) -> usize {
    match command {
        CompoundCommand::If(command) => {
            command
                .condition
                .iter_mut()
                .map(|command| rewrite_command_words(command, source, visitor))
                .sum::<usize>()
                + command
                    .then_branch
                    .iter_mut()
                    .map(|command| rewrite_command_words(command, source, visitor))
                    .sum::<usize>()
                + command
                    .elif_branches
                    .iter_mut()
                    .map(|(condition, body)| {
                        condition
                            .iter_mut()
                            .map(|command| rewrite_command_words(command, source, visitor))
                            .sum::<usize>()
                            + body
                                .iter_mut()
                                .map(|command| rewrite_command_words(command, source, visitor))
                                .sum::<usize>()
                    })
                    .sum::<usize>()
                + command
                    .else_branch
                    .iter_mut()
                    .flat_map(|body| body.iter_mut())
                    .map(|command| rewrite_command_words(command, source, visitor))
                    .sum::<usize>()
        }
        CompoundCommand::For(command) => {
            command
                .words
                .iter_mut()
                .flat_map(|words| words.iter_mut())
                .map(|word| walk_word(word, source, &mut |word| visitor(word, source)))
                .sum::<usize>()
                + command
                    .body
                    .iter_mut()
                    .map(|command| rewrite_command_words(command, source, visitor))
                    .sum::<usize>()
        }
        CompoundCommand::ArithmeticFor(command) => command
            .body
            .iter_mut()
            .map(|command| rewrite_command_words(command, source, visitor))
            .sum(),
        CompoundCommand::While(command) => {
            command
                .condition
                .iter_mut()
                .map(|command| rewrite_command_words(command, source, visitor))
                .sum::<usize>()
                + command
                    .body
                    .iter_mut()
                    .map(|command| rewrite_command_words(command, source, visitor))
                    .sum::<usize>()
        }
        CompoundCommand::Until(command) => {
            command
                .condition
                .iter_mut()
                .map(|command| rewrite_command_words(command, source, visitor))
                .sum::<usize>()
                + command
                    .body
                    .iter_mut()
                    .map(|command| rewrite_command_words(command, source, visitor))
                    .sum::<usize>()
        }
        CompoundCommand::Case(command) => {
            walk_word(&mut command.word, source, &mut |word| visitor(word, source))
                + command
                    .cases
                    .iter_mut()
                    .map(|item| {
                        item.patterns
                            .iter_mut()
                            .map(|pattern| {
                                walk_word(pattern, source, &mut |word| visitor(word, source))
                            })
                            .sum::<usize>()
                            + item
                                .commands
                                .iter_mut()
                                .map(|command| rewrite_command_words(command, source, visitor))
                                .sum::<usize>()
                    })
                    .sum::<usize>()
        }
        CompoundCommand::Select(command) => {
            command
                .words
                .iter_mut()
                .map(|word| walk_word(word, source, &mut |word| visitor(word, source)))
                .sum::<usize>()
                + command
                    .body
                    .iter_mut()
                    .map(|command| rewrite_command_words(command, source, visitor))
                    .sum::<usize>()
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .iter_mut()
            .map(|command| rewrite_command_words(command, source, visitor))
            .sum(),
        CompoundCommand::Arithmetic(_) => 0,
        CompoundCommand::Time(command) => command
            .command
            .as_mut()
            .map_or(0, |command| rewrite_command_words(command, source, visitor)),
        CompoundCommand::Conditional(command) => {
            walk_conditional_words(&mut command.expression, source, &mut |word| {
                visitor(word, source)
            })
        }
        CompoundCommand::Coproc(command) => {
            rewrite_command_words(command.body.as_mut(), source, visitor)
        }
    }
}

fn rewrite_assignment_words(
    assignment: &mut Assignment,
    source: &str,
    visitor: &mut impl FnMut(&mut Word, &str) -> usize,
) -> usize {
    match &mut assignment.value {
        AssignmentValue::Scalar(word) => walk_word(word, source, &mut |word| visitor(word, source)),
        AssignmentValue::Array(words) => words
            .iter_mut()
            .map(|word| walk_word(word, source, &mut |word| visitor(word, source)))
            .sum(),
    }
}

fn rewrite_redirect_words(
    redirect: &mut Redirect,
    source: &str,
    visitor: &mut impl FnMut(&mut Word, &str) -> usize,
) -> usize {
    match &mut redirect.target {
        RedirectTarget::Word(word) => walk_word(word, source, &mut |word| visitor(word, source)),
        RedirectTarget::Heredoc(heredoc) => {
            walk_word(&mut heredoc.delimiter.raw, source, &mut |word| {
                visitor(word, source)
            }) + walk_word(&mut heredoc.body, source, &mut |word| visitor(word, source))
        }
    }
}

fn walk_decl_name(
    name: &mut DeclName,
    _source: &str,
    visitor: &mut impl FnMut(&mut SourceText) -> usize,
) -> usize {
    name.index.as_mut().map_or(0, visitor)
}

fn walk_assignment(
    assignment: &mut Assignment,
    source: &str,
    source_text_visitor: &mut impl FnMut(&mut SourceText) -> usize,
    word_visitor: &mut impl FnMut(&mut Word) -> usize,
) -> usize {
    let mut count = assignment.index.as_mut().map_or(0, source_text_visitor);
    count += match &mut assignment.value {
        AssignmentValue::Scalar(word) => walk_word(word, source, word_visitor),
        AssignmentValue::Array(words) => {
            let mut inner = 0;
            for word in words {
                inner += walk_word(word, source, word_visitor);
            }
            inner
        }
    };
    count
}

fn walk_word(
    word: &mut Word,
    _source: &str,
    visitor: &mut impl FnMut(&mut Word) -> usize,
) -> usize {
    let mut count = visitor(word);
    for part in &mut word.parts {
        count += walk_word_part(part);
    }
    count
}

fn walk_word_part(part: &mut WordPartNode) -> usize {
    match &mut part.kind {
        WordPart::DoubleQuoted { parts, .. } => {
            let mut count = 0;
            for part in parts {
                count += walk_word_part(part);
            }
            count
        }
        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => 0,
        _ => 0,
    }
}

fn walk_conditional_words(
    expression: &mut ConditionalExpr,
    source: &str,
    visitor: &mut impl FnMut(&mut Word) -> usize,
) -> usize {
    match expression {
        ConditionalExpr::Binary(ConditionalBinaryExpr { left, right, .. }) => {
            walk_conditional_words(left, source, visitor)
                + walk_conditional_words(right, source, visitor)
        }
        ConditionalExpr::Unary(ConditionalUnaryExpr { expr, .. }) => {
            walk_conditional_words(expr, source, visitor)
        }
        ConditionalExpr::Parenthesized(ConditionalParenExpr { expr, .. }) => {
            walk_conditional_words(expr, source, visitor)
        }
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => walk_word(word, source, visitor),
    }
}

fn rewrite_command_source_texts(
    command: &mut Command,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match command {
        Command::Simple(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += rewrite_assignment_source_texts(assignment, source, visitor);
            }
            count += rewrite_word_source_texts(&mut command.name, source, visitor);
            for argument in &mut command.args {
                count += rewrite_word_source_texts(argument, source, visitor);
            }
            for redirect in &mut command.redirects {
                count += rewrite_redirect_source_texts(redirect, source, visitor);
            }
            count
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => rewrite_builtin_like_source_texts(
                &mut command.assignments,
                command.depth.as_mut(),
                &mut command.extra_args,
                &mut command.redirects,
                source,
                visitor,
            ),
            BuiltinCommand::Continue(command) => rewrite_builtin_like_source_texts(
                &mut command.assignments,
                command.depth.as_mut(),
                &mut command.extra_args,
                &mut command.redirects,
                source,
                visitor,
            ),
            BuiltinCommand::Return(command) => rewrite_builtin_like_source_texts(
                &mut command.assignments,
                command.code.as_mut(),
                &mut command.extra_args,
                &mut command.redirects,
                source,
                visitor,
            ),
            BuiltinCommand::Exit(command) => rewrite_builtin_like_source_texts(
                &mut command.assignments,
                command.code.as_mut(),
                &mut command.extra_args,
                &mut command.redirects,
                source,
                visitor,
            ),
        },
        Command::Decl(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += rewrite_assignment_source_texts(assignment, source, visitor);
            }
            for operand in &mut command.operands {
                count += match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        rewrite_word_source_texts(word, source, visitor)
                    }
                    DeclOperand::Name(name) => {
                        name.index.as_mut().map_or(0, |text| visitor(text, source))
                    }
                    DeclOperand::Assignment(assignment) => {
                        rewrite_assignment_source_texts(assignment, source, visitor)
                    }
                };
            }
            for redirect in &mut command.redirects {
                count += rewrite_redirect_source_texts(redirect, source, visitor);
            }
            count
        }
        Command::Pipeline(pipeline) => pipeline
            .commands
            .iter_mut()
            .map(|command| rewrite_command_source_texts(command, source, visitor))
            .sum(),
        Command::List(list) => {
            rewrite_command_source_texts(&mut list.first, source, visitor)
                + list
                    .rest
                    .iter_mut()
                    .map(|item| rewrite_command_source_texts(&mut item.command, source, visitor))
                    .sum::<usize>()
        }
        Command::Compound(compound, redirects) => {
            let mut count = rewrite_compound_source_texts(compound, source, visitor);
            for redirect in redirects {
                count += rewrite_redirect_source_texts(redirect, source, visitor);
            }
            count
        }
        Command::Function(function) => {
            rewrite_command_source_texts(function.body.as_mut(), source, visitor)
        }
    }
}

fn rewrite_builtin_like_source_texts(
    assignments: &mut [Assignment],
    primary: Option<&mut Word>,
    extra_args: &mut [Word],
    redirects: &mut [Redirect],
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    let mut count = 0;
    for assignment in assignments {
        count += rewrite_assignment_source_texts(assignment, source, visitor);
    }
    if let Some(primary) = primary {
        count += rewrite_word_source_texts(primary, source, visitor);
    }
    for argument in extra_args {
        count += rewrite_word_source_texts(argument, source, visitor);
    }
    for redirect in redirects {
        count += rewrite_redirect_source_texts(redirect, source, visitor);
    }
    count
}

fn rewrite_compound_source_texts(
    command: &mut CompoundCommand,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match command {
        CompoundCommand::If(command) => {
            command
                .condition
                .iter_mut()
                .map(|command| rewrite_command_source_texts(command, source, visitor))
                .sum::<usize>()
                + command
                    .then_branch
                    .iter_mut()
                    .map(|command| rewrite_command_source_texts(command, source, visitor))
                    .sum::<usize>()
                + command
                    .elif_branches
                    .iter_mut()
                    .map(|(condition, body)| {
                        condition
                            .iter_mut()
                            .map(|command| rewrite_command_source_texts(command, source, visitor))
                            .sum::<usize>()
                            + body
                                .iter_mut()
                                .map(|command| {
                                    rewrite_command_source_texts(command, source, visitor)
                                })
                                .sum::<usize>()
                    })
                    .sum::<usize>()
                + command
                    .else_branch
                    .iter_mut()
                    .flat_map(|body| body.iter_mut())
                    .map(|command| rewrite_command_source_texts(command, source, visitor))
                    .sum::<usize>()
        }
        CompoundCommand::For(command) => {
            command
                .words
                .iter_mut()
                .flat_map(|words| words.iter_mut())
                .map(|word| rewrite_word_source_texts(word, source, visitor))
                .sum::<usize>()
                + command
                    .body
                    .iter_mut()
                    .map(|command| rewrite_command_source_texts(command, source, visitor))
                    .sum::<usize>()
        }
        CompoundCommand::ArithmeticFor(command) => command
            .body
            .iter_mut()
            .map(|command| rewrite_command_source_texts(command, source, visitor))
            .sum(),
        CompoundCommand::While(command) => {
            command
                .condition
                .iter_mut()
                .map(|command| rewrite_command_source_texts(command, source, visitor))
                .sum::<usize>()
                + command
                    .body
                    .iter_mut()
                    .map(|command| rewrite_command_source_texts(command, source, visitor))
                    .sum::<usize>()
        }
        CompoundCommand::Until(command) => {
            command
                .condition
                .iter_mut()
                .map(|command| rewrite_command_source_texts(command, source, visitor))
                .sum::<usize>()
                + command
                    .body
                    .iter_mut()
                    .map(|command| rewrite_command_source_texts(command, source, visitor))
                    .sum::<usize>()
        }
        CompoundCommand::Case(command) => {
            rewrite_word_source_texts(&mut command.word, source, visitor)
                + command
                    .cases
                    .iter_mut()
                    .map(|item| {
                        item.patterns
                            .iter_mut()
                            .map(|pattern| rewrite_word_source_texts(pattern, source, visitor))
                            .sum::<usize>()
                            + item
                                .commands
                                .iter_mut()
                                .map(|command| {
                                    rewrite_command_source_texts(command, source, visitor)
                                })
                                .sum::<usize>()
                    })
                    .sum::<usize>()
        }
        CompoundCommand::Select(command) => {
            command
                .words
                .iter_mut()
                .map(|word| rewrite_word_source_texts(word, source, visitor))
                .sum::<usize>()
                + command
                    .body
                    .iter_mut()
                    .map(|command| rewrite_command_source_texts(command, source, visitor))
                    .sum::<usize>()
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => commands
            .iter_mut()
            .map(|command| rewrite_command_source_texts(command, source, visitor))
            .sum(),
        CompoundCommand::Arithmetic(_) => 0,
        CompoundCommand::Time(command) => command.command.as_mut().map_or(0, |command| {
            rewrite_command_source_texts(command, source, visitor)
        }),
        CompoundCommand::Conditional(command) => {
            rewrite_conditional_source_texts(command, source, visitor)
        }
        CompoundCommand::Coproc(command) => {
            rewrite_command_source_texts(command.body.as_mut(), source, visitor)
        }
    }
}

fn rewrite_conditional_source_texts(
    command: &mut ConditionalCommand,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    rewrite_conditional_expr_source_texts(&mut command.expression, source, visitor)
}

fn rewrite_conditional_expr_source_texts(
    expression: &mut ConditionalExpr,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match expression {
        ConditionalExpr::Binary(expr) => {
            rewrite_conditional_expr_source_texts(&mut expr.left, source, visitor)
                + rewrite_conditional_expr_source_texts(&mut expr.right, source, visitor)
        }
        ConditionalExpr::Unary(expr) => {
            rewrite_conditional_expr_source_texts(&mut expr.expr, source, visitor)
        }
        ConditionalExpr::Parenthesized(expr) => {
            rewrite_conditional_expr_source_texts(&mut expr.expr, source, visitor)
        }
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => rewrite_word_source_texts(word, source, visitor),
    }
}

fn rewrite_assignment_source_texts(
    assignment: &mut Assignment,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    let mut count = assignment
        .index
        .as_mut()
        .map_or(0, |text| visitor(text, source));
    count += match &mut assignment.value {
        AssignmentValue::Scalar(word) => rewrite_word_source_texts(word, source, visitor),
        AssignmentValue::Array(words) => words
            .iter_mut()
            .map(|word| rewrite_word_source_texts(word, source, visitor))
            .sum(),
    };
    count
}

fn rewrite_redirect_source_texts(
    redirect: &mut Redirect,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match &mut redirect.target {
        RedirectTarget::Word(word) => rewrite_word_source_texts(word, source, visitor),
        RedirectTarget::Heredoc(heredoc) => {
            rewrite_word_source_texts(&mut heredoc.delimiter.raw, source, visitor)
                + rewrite_word_source_texts(&mut heredoc.body, source, visitor)
        }
    }
}

fn rewrite_word_source_texts(
    word: &mut Word,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    let mut count = 0;
    for part in &mut word.parts {
        count += match &mut part.kind {
            WordPart::DoubleQuoted { parts, .. } => parts
                .iter_mut()
                .map(|part| rewrite_word_part_source_texts(part, source, visitor))
                .sum(),
            _ => rewrite_word_part_source_texts(part, source, visitor),
        };
    }
    count
}

fn rewrite_word_part_source_texts(
    part: &mut WordPartNode,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match &mut part.kind {
        WordPart::Literal(_) | WordPart::Variable(_) | WordPart::Length(_) => 0,
        WordPart::SingleQuoted { value, .. } => visitor(value, source),
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter_mut()
            .map(|part| rewrite_word_part_source_texts(part, source, visitor))
            .sum(),
        WordPart::CommandSubstitution { commands, .. }
        | WordPart::ProcessSubstitution { commands, .. } => commands
            .iter_mut()
            .map(|command| rewrite_command_source_texts(command, source, visitor))
            .sum(),
        WordPart::ArithmeticExpansion { expression, .. } => visitor(expression, source),
        WordPart::ParameterExpansion {
            operator, operand, ..
        } => {
            operand
                .as_mut()
                .map_or(0, |operand| visitor(operand, source))
                + rewrite_parameter_op_source_texts(operator, source, visitor)
        }
        WordPart::ArrayAccess { index, .. } => visitor(index, source),
        WordPart::ArrayLength(_) | WordPart::ArrayIndices(_) => 0,
        WordPart::Substring { offset, length, .. } => {
            visitor(offset, source) + length.as_mut().map_or(0, |length| visitor(length, source))
        }
        WordPart::ArraySlice { offset, length, .. } => {
            visitor(offset, source) + length.as_mut().map_or(0, |length| visitor(length, source))
        }
        WordPart::IndirectExpansion {
            operand, operator, ..
        } => {
            operand
                .as_mut()
                .map_or(0, |operand| visitor(operand, source))
                + operator.as_ref().map_or(0, |_| 0)
        }
        WordPart::PrefixMatch(_) | WordPart::Transformation { .. } => 0,
    }
}

fn rewrite_parameter_op_source_texts(
    operator: &mut ParameterOp,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match operator {
        ParameterOp::ReplaceFirst {
            pattern,
            replacement,
        }
        | ParameterOp::ReplaceAll {
            pattern,
            replacement,
        } => visitor(pattern, source) + visitor(replacement, source),
        _ => 0,
    }
}

fn transform_source_text(
    text: &mut SourceText,
    source: &str,
    transform: impl FnOnce(&str) -> Option<String>,
) -> usize {
    let current = text.slice(source);
    let Some(next) = transform(current) else {
        return 0;
    };
    if next == current {
        return 0;
    }
    *text = SourceText::cooked(text.span(), next);
    1
}

fn strip_single_outer_parens(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if !trimmed.starts_with('(') || !trimmed.ends_with(')') {
        return None;
    }

    let mut depth = 0usize;
    for (index, ch) in trimmed.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && index + ch.len_utf8() != trimmed.len() {
                    return None;
                }
            }
            _ => {}
        }
    }

    if depth != 0 {
        return None;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    (!inner.is_empty() && !inner.contains('\n')).then(|| inner.to_string())
}

fn simplify_arithmetic_variables_text(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if let Some(name) = trimmed.strip_prefix('$')
        && !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '[' | ']'))
        && !name.chars().all(|ch| ch.is_ascii_digit())
    {
        return None;
    }

    let mut output = String::with_capacity(text.len());
    let mut changed = false;
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;

    while index < chars.len() {
        if chars[index] != '$' {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        if index + 1 >= chars.len() {
            output.push(chars[index]);
            index += 1;
            continue;
        }

        if chars[index + 1] == '{' {
            let mut end = index + 2;
            while end < chars.len() && chars[end] != '}' {
                end += 1;
            }
            if end < chars.len() {
                let inner: String = chars[index + 2..end].iter().collect();
                if arithmetic_parameter_is_safe(&inner) {
                    output.push_str(&inner);
                    changed = true;
                    index = end + 1;
                    continue;
                }
            }
        } else if chars[index + 1].is_ascii_alphabetic() || chars[index + 1] == '_' {
            let mut end = index + 2;
            while end < chars.len()
                && (chars[end].is_ascii_alphanumeric()
                    || chars[end] == '_'
                    || chars[end] == '['
                    || chars[end] == ']')
            {
                end += 1;
            }
            let ident: String = chars[index + 1..end].iter().collect();
            output.push_str(&ident);
            changed = true;
            index = end;
            continue;
        }

        output.push(chars[index]);
        index += 1;
    }

    changed.then_some(output)
}

fn arithmetic_parameter_is_safe(text: &str) -> bool {
    !text.is_empty()
        && !matches!(text.chars().next(), Some('!' | '#'))
        && !text.chars().all(|ch| ch.is_ascii_digit())
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '[' | ']'))
}

fn simplify_conditional_command(command: &mut ConditionalCommand, source: &str) -> usize {
    simplify_conditional_expr(&mut command.expression, source)
}

fn simplify_conditional_expr(expression: &mut ConditionalExpr, source: &str) -> usize {
    let mut changes = match expression {
        ConditionalExpr::Binary(expr) => {
            let mut count = simplify_conditional_expr(&mut expr.left, source)
                + simplify_conditional_expr(&mut expr.right, source);

            if expr.op == ConditionalBinaryOp::PatternEqShort {
                expr.op = ConditionalBinaryOp::PatternEq;
                count += 1;
            }

            count += strip_redundant_conditional_quotes(&mut expr.left, source);
            count += strip_redundant_conditional_quotes(&mut expr.right, source);
            count
        }
        ConditionalExpr::Unary(expr) => {
            let mut count = simplify_conditional_expr(&mut expr.expr, source);
            count += strip_redundant_conditional_quotes(&mut expr.expr, source);
            count
        }
        ConditionalExpr::Parenthesized(expr) => simplify_conditional_expr(&mut expr.expr, source),
        ConditionalExpr::Word(_) | ConditionalExpr::Pattern(_) | ConditionalExpr::Regex(_) => 0,
    };

    loop {
        let Some(next) = simplify_conditional_node(expression) else {
            break;
        };
        *expression = next;
        changes += 1;
    }

    changes
}

fn simplify_conditional_node(expression: &ConditionalExpr) -> Option<ConditionalExpr> {
    match expression {
        ConditionalExpr::Unary(expr) if expr.op == ConditionalUnaryOp::Not => {
            match expr.expr.as_ref() {
                ConditionalExpr::Unary(inner) if inner.op == ConditionalUnaryOp::Not => {
                    Some((*inner.expr).clone())
                }
                ConditionalExpr::Unary(inner) if inner.op == ConditionalUnaryOp::NonEmptyString => {
                    Some(ConditionalExpr::Unary(ConditionalUnaryExpr {
                        op: ConditionalUnaryOp::EmptyString,
                        op_span: inner.op_span,
                        expr: inner.expr.clone(),
                    }))
                }
                ConditionalExpr::Unary(inner) if inner.op == ConditionalUnaryOp::EmptyString => {
                    Some(ConditionalExpr::Unary(ConditionalUnaryExpr {
                        op: ConditionalUnaryOp::NonEmptyString,
                        op_span: inner.op_span,
                        expr: inner.expr.clone(),
                    }))
                }
                ConditionalExpr::Binary(inner) => invert_conditional_binary(inner),
                ConditionalExpr::Parenthesized(inner) => Some((*inner.expr).clone()),
                _ => None,
            }
        }
        ConditionalExpr::Parenthesized(expr)
            if matches!(
                expr.expr.as_ref(),
                ConditionalExpr::Unary(_)
                    | ConditionalExpr::Word(_)
                    | ConditionalExpr::Pattern(_)
                    | ConditionalExpr::Regex(_)
            ) =>
        {
            Some((*expr.expr).clone())
        }
        _ => None,
    }
}

fn invert_conditional_binary(expression: &ConditionalBinaryExpr) -> Option<ConditionalExpr> {
    let op = match expression.op {
        ConditionalBinaryOp::PatternEqShort | ConditionalBinaryOp::PatternEq => {
            ConditionalBinaryOp::PatternNe
        }
        ConditionalBinaryOp::PatternNe => ConditionalBinaryOp::PatternEq,
        _ => return None,
    };
    Some(ConditionalExpr::Binary(ConditionalBinaryExpr {
        left: expression.left.clone(),
        op,
        op_span: expression.op_span,
        right: expression.right.clone(),
    }))
}

fn strip_redundant_conditional_quotes(expression: &mut ConditionalExpr, source: &str) -> usize {
    let ConditionalExpr::Word(word) = expression else {
        return 0;
    };

    if !word_is_redundantly_quoted_variable(word) {
        return 0;
    }

    let Some(WordPart::DoubleQuoted { parts, .. }) = word.parts.first().map(|part| &part.kind)
    else {
        return 0;
    };
    let parts = parts.clone();
    *word = Word {
        parts,
        span: word.span,
    };
    let _ = source;
    1
}

fn word_is_redundantly_quoted_variable(word: &Word) -> bool {
    matches!(
        word.parts.as_slice(),
        [WordPartNode {
            kind: WordPart::DoubleQuoted { parts, dollar: false },
            ..
        }] if matches!(parts.as_slice(), [WordPartNode { kind: WordPart::Variable(_), .. }])
    )
}

fn tighten_literal_quotes(word: &mut Word, source: &str) -> usize {
    let Some(part) = word.parts.first_mut() else {
        return 0;
    };

    let WordPart::DoubleQuoted { parts, dollar } = &part.kind else {
        return 0;
    };
    if *dollar {
        return 0;
    }

    let Some(literal) = literal_text_from_double_quoted(parts, source) else {
        return 0;
    };
    if literal.is_empty() || literal.contains('\'') || literal.contains('\n') {
        return 0;
    }

    *word = Word {
        parts: vec![WordPartNode::new(
            WordPart::SingleQuoted {
                value: SourceText::cooked(word.span, literal),
                dollar: false,
            },
            word.span,
        )],
        span: word.span,
    };
    1
}

fn literal_text_from_double_quoted(parts: &[WordPartNode], source: &str) -> Option<String> {
    let mut value = String::new();
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => {
                let raw = text.as_str(source, part.span);
                if text.is_source_backed() && raw.contains('\\') {
                    return None;
                }
                value.push_str(raw);
            }
            _ => return None,
        }
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use shuck_format::format;
    use shuck_parser::parser::Parser;

    use crate::comments::Comments;
    use crate::context::ShellFormatContext;
    use crate::options::ShellFormatOptions;
    use crate::shared_traits::AsFormat;

    use super::*;

    fn rewrite_by_name(name: &str) -> SimplifyRewrite {
        *REWRITES
            .iter()
            .find(|rewrite| rewrite.name == name)
            .unwrap_or_else(|| panic!("missing rewrite {name}"))
    }

    fn format_after_rewrite(source: &str, rewrite: &str) -> String {
        let mut parsed = Parser::new(source).parse().unwrap();
        let rewrite = rewrite_by_name(rewrite);
        let changes = (rewrite.apply)(&mut parsed.script, source);
        assert!(changes > 0, "expected rewrite `{rewrite:?}` to apply");

        let options = ShellFormatOptions::default();
        let context = ShellFormatContext::new(
            options.resolve(source, Some(Path::new("test.sh"))),
            source,
            Comments::from_ast(source, &parsed.comments),
        );
        let formatted = format!(context, [parsed.script.format()]).unwrap();
        let mut output = formatted.print().unwrap().into_code();
        if !output.ends_with('\n') {
            output.push('\n');
        }
        output
    }

    fn format_with_simplify(source: &str) -> String {
        match crate::format_source(
            source,
            Some(Path::new("test.sh")),
            &ShellFormatOptions::default().with_simplify(true),
        )
        .unwrap()
        {
            crate::FormattedSource::Unchanged => source.to_string(),
            crate::FormattedSource::Formatted(formatted) => formatted,
        }
    }

    #[test]
    fn paren_cleanup_simplifies_index_fragments() {
        assert_eq!(
            format_after_rewrite("echo ${foo[(1)]}\n", "paren-cleanup"),
            "echo ${foo[1]}\n"
        );
    }

    #[test]
    fn paren_cleanup_skips_dynamic_indexes() {
        assert_eq!(
            format_with_simplify("echo ${foo[$bar]}\n"),
            "echo ${foo[$bar]}\n"
        );
    }

    #[test]
    fn arithmetic_var_rewrite_unwraps_simple_parameters() {
        assert_eq!(
            format_after_rewrite("echo $(( $a + ${b} ))\n", "arithmetic-vars"),
            "echo $(( a + b ))\n"
        );
    }

    #[test]
    fn arithmetic_var_rewrite_skips_special_parameters() {
        assert_eq!(
            format_with_simplify("echo $(( ${!a} + ${#b} ))\n"),
            "echo $(( ${!a} + ${#b} ))\n"
        );
    }

    #[test]
    fn conditional_rewrite_normalizes_not_and_short_equals() {
        assert_eq!(
            format_after_rewrite("[[ ! -n \"$foo\" ]]\n", "conditionals"),
            "[[ -z $foo ]]\n"
        );
        assert_eq!(
            format_after_rewrite("[[ foo = bar ]]\n", "conditionals"),
            "[[ foo == bar ]]\n"
        );
    }

    #[test]
    fn quote_tightening_rewrites_simple_literal_quotes() {
        assert_eq!(
            format_after_rewrite("echo \"fo\\$o\"\n", "quote-tightening"),
            "echo 'fo$o'\n"
        );
    }

    #[test]
    fn quote_tightening_skips_mixed_expansions() {
        assert_eq!(
            format_with_simplify("echo \"$foo bar\"\n"),
            "echo \"$foo bar\"\n"
        );
    }

    #[test]
    fn simplify_report_tracks_applied_rewrites() {
        let mut parsed = Parser::new("echo $(( $a + ${b} ))\n").parse().unwrap();
        let report = simplify_script(&mut parsed.script, "echo $(( $a + ${b} ))\n");

        assert_eq!(report.total_changes(), 1);
        assert_eq!(report.applied().len(), 1);
        assert_eq!(report.applied()[0].name, "arithmetic-vars");
        assert_eq!(report.applied()[0].changes, 1);
    }
}
