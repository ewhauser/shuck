use shuck_ast::{
    ArrayElem, Assignment, AssignmentValue, BourneParameterExpansion, BuiltinCommand, Command,
    CompoundCommand, ConditionalBinaryExpr, ConditionalBinaryOp, ConditionalCommand,
    ConditionalExpr, ConditionalParenExpr, ConditionalUnaryExpr, ConditionalUnaryOp, DeclClause,
    DeclOperand, File, FunctionDef, HeredocBody, HeredocBodyPart, HeredocBodyPartNode,
    ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern, PatternPart, Redirect,
    RedirectTarget, SourceText, Stmt, StmtSeq, VarRef, Word, WordPart, WordPartNode,
    ZshExpansionOperation, ZshExpansionTarget,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimplifyReport {
    applied: Vec<RewriteApplication>,
}

#[allow(dead_code)]
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
    pub apply: fn(&mut File, &str) -> usize,
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

pub fn simplify_file(file: &mut File, source: &str) -> SimplifyReport {
    let mut applied = Vec::new();
    for rewrite in REWRITES {
        let changes = (rewrite.apply)(file, source);
        if changes > 0 {
            applied.push(RewriteApplication {
                name: rewrite.name,
                changes,
            });
        }
    }
    SimplifyReport { applied }
}

fn rewrite_paren_cleanup(file: &mut File, source: &str) -> usize {
    walk_stmts(file, source, &mut |stmt, source| {
        let mut changes = 0;
        changes += rewrite_stmt_source_texts(stmt, source, &mut |text, source| {
            transform_source_text(text, source, strip_single_outer_parens)
        });
        changes
    })
}

fn rewrite_arithmetic_variables(file: &mut File, source: &str) -> usize {
    walk_stmts(file, source, &mut |stmt, source| {
        rewrite_stmt_source_texts(stmt, source, &mut |text, source| {
            transform_source_text(text, source, simplify_arithmetic_variables_text)
        })
    })
}

fn rewrite_conditionals(file: &mut File, source: &str) -> usize {
    walk_stmts(file, source, &mut |stmt, source| match &mut stmt.command {
        Command::Compound(CompoundCommand::Conditional(conditional)) => {
            simplify_conditional_command(conditional, source)
        }
        _ => 0,
    })
}

fn rewrite_nested_subshells(file: &mut File, source: &str) -> usize {
    walk_stmts(file, source, &mut |stmt, source| {
        let mut changes = 0;
        if let Command::Compound(CompoundCommand::Subshell(commands)) = &mut stmt.command
            && !stmt.negated
            && stmt.redirects.is_empty()
            && stmt.terminator.is_none()
        {
            changes += collapse_nested_subshell_sequence(commands);
        }

        changes += rewrite_stmt_words(stmt, source, &mut |word, _source| {
            let mut word_changes = 0;
            for part in &mut word.parts {
                match &mut part.kind {
                    WordPart::CommandSubstitution { body, .. }
                    | WordPart::ProcessSubstitution { body, .. } => {
                        word_changes += collapse_nested_subshell_sequence(body);
                    }
                    _ => {}
                }
            }
            word_changes
        });
        changes
    })
}

fn rewrite_quote_tightening(file: &mut File, source: &str) -> usize {
    walk_stmts(file, source, &mut |stmt, source| {
        rewrite_stmt_words(stmt, source, &mut |word, source| {
            tighten_literal_quotes(word, source)
        })
    })
}

fn collapse_nested_subshell_sequence(commands: &mut StmtSeq) -> usize {
    let mut changes = 0;
    while commands.leading_comments.is_empty()
        && commands.trailing_comments.is_empty()
        && commands.len() == 1
    {
        let Some(Stmt {
            leading_comments,
            command: Command::Compound(CompoundCommand::Subshell(inner)),
            negated: false,
            redirects,
            terminator: None,
            inline_comment: None,
            ..
        }) = commands.first()
        else {
            break;
        };
        if !leading_comments.is_empty() || !redirects.is_empty() {
            break;
        }
        *commands = inner.clone();
        changes += 1;
    }
    changes
}

fn walk_stmts(
    file: &mut File,
    source: &str,
    visitor: &mut impl FnMut(&mut Stmt, &str) -> usize,
) -> usize {
    walk_stmt_seq(&mut file.body, source, visitor)
}

fn walk_stmt_seq(
    commands: &mut StmtSeq,
    source: &str,
    visitor: &mut impl FnMut(&mut Stmt, &str) -> usize,
) -> usize {
    commands
        .iter_mut()
        .map(|command| walk_stmt(command, source, visitor))
        .sum()
}

fn walk_stmt(
    stmt: &mut Stmt,
    source: &str,
    visitor: &mut impl FnMut(&mut Stmt, &str) -> usize,
) -> usize {
    let mut changes = visitor(stmt, source);
    changes += match &mut stmt.command {
        Command::Simple(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += walk_assignment(assignment, source, &mut |_| 0, &mut |_| 0);
            }
            count += walk_word(&mut command.name, source, &mut |_| 0);
            for argument in &mut command.args {
                count += walk_word(argument, source, &mut |_| 0);
            }
            count
        }
        Command::Builtin(command) => walk_builtin(command, source),
        Command::Decl(command) => walk_decl_clause(command, source),
        Command::Binary(command) => {
            walk_stmt(&mut command.left, source, visitor)
                + walk_stmt(&mut command.right, source, visitor)
        }
        Command::Compound(compound) => walk_compound(compound, source, visitor),
        Command::Function(function) => walk_function(function, source, visitor),
        Command::AnonymousFunction(function) => walk_anonymous_function(function, source, visitor),
    };
    for redirect in &mut stmt.redirects {
        changes += walk_redirect(redirect, source, &mut |_| 0);
    }
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
            DeclOperand::Name(name) => walk_var_ref(name, source, &mut |_| 0),
            DeclOperand::Assignment(assignment) => {
                walk_assignment(assignment, source, &mut |_| 0, &mut |_| 0)
            }
        };
    }
    count
}

fn walk_function(
    function: &mut FunctionDef,
    source: &str,
    visitor: &mut impl FnMut(&mut Stmt, &str) -> usize,
) -> usize {
    let mut count = 0;
    for entry in &mut function.header.entries {
        count += walk_word(&mut entry.word, source, &mut |_| 0);
    }
    count + walk_stmt(function.body.as_mut(), source, visitor)
}

fn walk_anonymous_function(
    function: &mut shuck_ast::AnonymousFunctionCommand,
    source: &str,
    visitor: &mut impl FnMut(&mut Stmt, &str) -> usize,
) -> usize {
    let mut count = walk_stmt(function.body.as_mut(), source, visitor);
    for argument in &mut function.args {
        count += walk_word(argument, source, &mut |_| 0);
    }
    count
}

fn walk_compound(
    command: &mut CompoundCommand,
    source: &str,
    visitor: &mut impl FnMut(&mut Stmt, &str) -> usize,
) -> usize {
    match command {
        CompoundCommand::If(command) => {
            let mut count = 0;
            count += walk_stmt_seq(&mut command.condition, source, visitor);
            count += walk_stmt_seq(&mut command.then_branch, source, visitor);
            for (condition, body) in &mut command.elif_branches {
                count += walk_stmt_seq(condition, source, visitor);
                count += walk_stmt_seq(body, source, visitor);
            }
            if let Some(body) = &mut command.else_branch {
                count += walk_stmt_seq(body, source, visitor);
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
            count + walk_stmt_seq(&mut command.body, source, visitor)
        }
        CompoundCommand::Repeat(command) => {
            walk_word(&mut command.count, source, &mut |_| 0)
                + walk_stmt_seq(&mut command.body, source, visitor)
        }
        CompoundCommand::Foreach(command) => {
            command
                .words
                .iter_mut()
                .map(|word| walk_word(word, source, &mut |_| 0))
                .sum::<usize>()
                + walk_stmt_seq(&mut command.body, source, visitor)
        }
        CompoundCommand::ArithmeticFor(command) => {
            walk_stmt_seq(&mut command.body, source, visitor)
        }
        CompoundCommand::While(command) => {
            walk_stmt_seq(&mut command.condition, source, visitor)
                + walk_stmt_seq(&mut command.body, source, visitor)
        }
        CompoundCommand::Until(command) => {
            walk_stmt_seq(&mut command.condition, source, visitor)
                + walk_stmt_seq(&mut command.body, source, visitor)
        }
        CompoundCommand::Case(command) => {
            let mut count = walk_word(&mut command.word, source, &mut |_| 0);
            for item in &mut command.cases {
                for pattern in &mut item.patterns {
                    count += walk_pattern(pattern, source, &mut |_| 0);
                }
                count += walk_stmt_seq(&mut item.body, source, visitor);
            }
            count
        }
        CompoundCommand::Select(command) => {
            let mut count = 0;
            for word in &mut command.words {
                count += walk_word(word, source, &mut |_| 0);
            }
            count + walk_stmt_seq(&mut command.body, source, visitor)
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            walk_stmt_seq(commands, source, visitor)
        }
        CompoundCommand::Arithmetic(_) => 0,
        CompoundCommand::Time(command) => command
            .command
            .as_mut()
            .map_or(0, |command| walk_stmt(command, source, visitor)),
        CompoundCommand::Conditional(command) => {
            let mut count = 0;
            count += walk_conditional_words(&mut command.expression, source, &mut |_| 0);
            count
        }
        CompoundCommand::Coproc(command) => walk_stmt(command.body.as_mut(), source, visitor),
        CompoundCommand::Always(command) => {
            walk_stmt_seq(&mut command.body, source, visitor)
                + walk_stmt_seq(&mut command.always_body, source, visitor)
        }
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
                + walk_heredoc_body(&mut heredoc.body, source, word_visitor)
        }
    }
}

fn rewrite_stmt_words(
    stmt: &mut Stmt,
    source: &str,
    visitor: &mut impl FnMut(&mut Word, &str) -> usize,
) -> usize {
    let mut count = match &mut stmt.command {
        Command::Simple(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += rewrite_assignment_words(assignment, source, visitor);
            }
            count += walk_word(&mut command.name, source, &mut |word| visitor(word, source));
            for argument in &mut command.args {
                count += walk_word(argument, source, &mut |word| visitor(word, source));
            }
            count
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => rewrite_builtin_like_words(
                &mut command.assignments,
                command.depth.as_mut(),
                &mut command.extra_args,
                source,
                visitor,
            ),
            BuiltinCommand::Continue(command) => rewrite_builtin_like_words(
                &mut command.assignments,
                command.depth.as_mut(),
                &mut command.extra_args,
                source,
                visitor,
            ),
            BuiltinCommand::Return(command) => rewrite_builtin_like_words(
                &mut command.assignments,
                command.code.as_mut(),
                &mut command.extra_args,
                source,
                visitor,
            ),
            BuiltinCommand::Exit(command) => rewrite_builtin_like_words(
                &mut command.assignments,
                command.code.as_mut(),
                &mut command.extra_args,
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
            count
        }
        Command::Binary(command) => {
            rewrite_stmt_words(&mut command.left, source, visitor)
                + rewrite_stmt_words(&mut command.right, source, visitor)
        }
        Command::Compound(compound) => rewrite_compound_words(compound, source, visitor),
        Command::Function(function) => {
            let mut count = 0;
            for entry in &mut function.header.entries {
                count += walk_word(&mut entry.word, source, &mut |word| visitor(word, source));
            }
            count + rewrite_stmt_words(function.body.as_mut(), source, visitor)
        }
        Command::AnonymousFunction(function) => {
            let mut count = rewrite_stmt_words(function.body.as_mut(), source, visitor);
            for argument in &mut function.args {
                count += walk_word(argument, source, &mut |word| visitor(word, source));
            }
            count
        }
    };
    for redirect in &mut stmt.redirects {
        count += rewrite_redirect_words(redirect, source, visitor);
    }
    count
}

fn rewrite_builtin_like_words(
    assignments: &mut [Assignment],
    primary: Option<&mut Word>,
    extra_args: &mut [Word],
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
    count
}

fn rewrite_compound_words(
    command: &mut CompoundCommand,
    source: &str,
    visitor: &mut impl FnMut(&mut Word, &str) -> usize,
) -> usize {
    match command {
        CompoundCommand::If(command) => {
            let mut count = rewrite_stmt_seq_words(&mut command.condition, source, visitor)
                + rewrite_stmt_seq_words(&mut command.then_branch, source, visitor);

            for (condition, body) in &mut command.elif_branches {
                count += rewrite_stmt_seq_words(condition, source, visitor);
                count += rewrite_stmt_seq_words(body, source, visitor);
            }

            if let Some(body) = &mut command.else_branch {
                count += rewrite_stmt_seq_words(body, source, visitor);
            }

            count
        }
        CompoundCommand::For(command) => {
            let mut count = 0;
            if let Some(words) = &mut command.words {
                for word in words {
                    count += walk_word(word, source, &mut |word| visitor(word, source));
                }
            }
            count + rewrite_stmt_seq_words(&mut command.body, source, visitor)
        }
        CompoundCommand::Repeat(command) => {
            walk_word(&mut command.count, source, &mut |word| {
                visitor(word, source)
            }) + rewrite_stmt_seq_words(&mut command.body, source, visitor)
        }
        CompoundCommand::Foreach(command) => {
            let mut count = 0;
            for word in &mut command.words {
                count += walk_word(word, source, &mut |word| visitor(word, source));
            }
            count + rewrite_stmt_seq_words(&mut command.body, source, visitor)
        }
        CompoundCommand::ArithmeticFor(command) => {
            rewrite_stmt_seq_words(&mut command.body, source, visitor)
        }
        CompoundCommand::While(command) => {
            rewrite_stmt_seq_words(&mut command.condition, source, visitor)
                + rewrite_stmt_seq_words(&mut command.body, source, visitor)
        }
        CompoundCommand::Until(command) => {
            rewrite_stmt_seq_words(&mut command.condition, source, visitor)
                + rewrite_stmt_seq_words(&mut command.body, source, visitor)
        }
        CompoundCommand::Case(command) => {
            let mut count = walk_word(&mut command.word, source, &mut |word| visitor(word, source));
            for item in &mut command.cases {
                for pattern in &mut item.patterns {
                    count += walk_pattern(pattern, source, &mut |word| visitor(word, source));
                }
                count += rewrite_stmt_seq_words(&mut item.body, source, visitor);
            }
            count
        }
        CompoundCommand::Select(command) => {
            let mut count = 0;
            for word in &mut command.words {
                count += walk_word(word, source, &mut |word| visitor(word, source));
            }
            count + rewrite_stmt_seq_words(&mut command.body, source, visitor)
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            rewrite_stmt_seq_words(commands, source, visitor)
        }
        CompoundCommand::Arithmetic(_) => 0,
        CompoundCommand::Time(command) => command
            .command
            .as_mut()
            .map_or(0, |command| rewrite_stmt_words(command, source, visitor)),
        CompoundCommand::Conditional(command) => {
            walk_conditional_words(&mut command.expression, source, &mut |word| {
                visitor(word, source)
            })
        }
        CompoundCommand::Coproc(command) => {
            rewrite_stmt_words(command.body.as_mut(), source, visitor)
        }
        CompoundCommand::Always(command) => {
            rewrite_stmt_seq_words(&mut command.body, source, visitor)
                + rewrite_stmt_seq_words(&mut command.always_body, source, visitor)
        }
    }
}

fn rewrite_stmt_seq_words(
    commands: &mut StmtSeq,
    source: &str,
    visitor: &mut impl FnMut(&mut Word, &str) -> usize,
) -> usize {
    let mut count = 0;
    for command in commands.iter_mut() {
        count += rewrite_stmt_words(command, source, visitor);
    }
    count
}

fn rewrite_assignment_words(
    assignment: &mut Assignment,
    source: &str,
    visitor: &mut impl FnMut(&mut Word, &str) -> usize,
) -> usize {
    match &mut assignment.value {
        AssignmentValue::Scalar(word) => walk_word(word, source, &mut |word| visitor(word, source)),
        AssignmentValue::Compound(array) => {
            let mut count = 0;
            for element in &mut array.elements {
                count += match element {
                    ArrayElem::Sequential(word)
                    | ArrayElem::Keyed { value: word, .. }
                    | ArrayElem::KeyedAppend { value: word, .. } => {
                        walk_word(word, source, &mut |word| visitor(word, source))
                    }
                };
            }
            count
        }
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
            }) + walk_heredoc_body(&mut heredoc.body, source, &mut |word| visitor(word, source))
        }
    }
}

fn walk_var_ref(
    name: &mut VarRef,
    _source: &str,
    visitor: &mut impl FnMut(&mut SourceText) -> usize,
) -> usize {
    name.subscript.as_mut().map_or(0, |subscript| {
        let mut count = visitor(&mut subscript.text);
        if let Some(raw) = &mut subscript.raw {
            count += visitor(raw);
        }
        count
    })
}

fn walk_assignment(
    assignment: &mut Assignment,
    source: &str,
    source_text_visitor: &mut impl FnMut(&mut SourceText) -> usize,
    word_visitor: &mut impl FnMut(&mut Word) -> usize,
) -> usize {
    let mut count = walk_var_ref(&mut assignment.target, source, source_text_visitor);
    count += match &mut assignment.value {
        AssignmentValue::Scalar(word) => walk_word(word, source, word_visitor),
        AssignmentValue::Compound(array) => {
            let mut inner = 0;
            for element in &mut array.elements {
                match element {
                    ArrayElem::Sequential(word)
                    | ArrayElem::Keyed { value: word, .. }
                    | ArrayElem::KeyedAppend { value: word, .. } => {
                        inner += walk_word(word, source, word_visitor);
                    }
                }
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

fn walk_heredoc_body(
    body: &mut HeredocBody,
    source: &str,
    visitor: &mut dyn FnMut(&mut Word) -> usize,
) -> usize {
    let mut count = 0;

    for part in &mut body.parts {
        if let HeredocBodyPart::CommandSubstitution {
            body: command_body, ..
        } = &mut part.kind
        {
            count +=
                rewrite_stmt_seq_words(command_body, source, &mut |word, _source| visitor(word));
        }
    }

    if count > 0 {
        body.source_backed = false;
    }

    count
}

fn walk_word_part(part: &mut WordPartNode) -> usize {
    match &mut part.kind {
        WordPart::ZshQualifiedGlob(_) => 0,
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

fn walk_pattern(
    pattern: &mut Pattern,
    source: &str,
    visitor: &mut impl FnMut(&mut Word) -> usize,
) -> usize {
    let mut count = 0;

    for part in &mut pattern.parts {
        count += match &mut part.kind {
            PatternPart::Group { patterns, .. } => {
                let mut inner = 0;
                for pattern in patterns {
                    inner += walk_pattern(pattern, source, visitor);
                }
                inner
            }
            PatternPart::Word(word) => walk_word(word, source, visitor),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => 0,
        };
    }

    count
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
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            walk_word(word, source, visitor)
        }
        ConditionalExpr::Pattern(pattern) => walk_pattern(pattern, source, visitor),
        ConditionalExpr::VarRef(_) => 0,
    }
}

fn rewrite_stmt_source_texts(
    stmt: &mut Stmt,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    let mut count = match &mut stmt.command {
        Command::Simple(command) => {
            let mut count = 0;
            for assignment in &mut command.assignments {
                count += rewrite_assignment_source_texts(assignment, source, visitor);
            }
            count += rewrite_word_source_texts(&mut command.name, source, visitor);
            for argument in &mut command.args {
                count += rewrite_word_source_texts(argument, source, visitor);
            }
            count
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => rewrite_builtin_like_source_texts(
                &mut command.assignments,
                command.depth.as_mut(),
                &mut command.extra_args,
                source,
                visitor,
            ),
            BuiltinCommand::Continue(command) => rewrite_builtin_like_source_texts(
                &mut command.assignments,
                command.depth.as_mut(),
                &mut command.extra_args,
                source,
                visitor,
            ),
            BuiltinCommand::Return(command) => rewrite_builtin_like_source_texts(
                &mut command.assignments,
                command.code.as_mut(),
                &mut command.extra_args,
                source,
                visitor,
            ),
            BuiltinCommand::Exit(command) => rewrite_builtin_like_source_texts(
                &mut command.assignments,
                command.code.as_mut(),
                &mut command.extra_args,
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
                    DeclOperand::Name(name) => rewrite_var_ref_source_texts(name, source, visitor),
                    DeclOperand::Assignment(assignment) => {
                        rewrite_assignment_source_texts(assignment, source, visitor)
                    }
                };
            }
            count
        }
        Command::Binary(command) => {
            rewrite_stmt_source_texts(&mut command.left, source, visitor)
                + rewrite_stmt_source_texts(&mut command.right, source, visitor)
        }
        Command::Compound(compound) => rewrite_compound_source_texts(compound, source, visitor),
        Command::Function(function) => {
            let mut count = 0;
            for entry in &mut function.header.entries {
                count += rewrite_word_source_texts(&mut entry.word, source, visitor);
            }
            count + rewrite_stmt_source_texts(function.body.as_mut(), source, visitor)
        }
        Command::AnonymousFunction(function) => {
            let mut count = rewrite_stmt_source_texts(function.body.as_mut(), source, visitor);
            for argument in &mut function.args {
                count += rewrite_word_source_texts(argument, source, visitor);
            }
            count
        }
    };
    for redirect in &mut stmt.redirects {
        count += rewrite_redirect_source_texts(redirect, source, visitor);
    }
    count
}

fn rewrite_builtin_like_source_texts(
    assignments: &mut [Assignment],
    primary: Option<&mut Word>,
    extra_args: &mut [Word],
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
    count
}

fn rewrite_compound_source_texts(
    command: &mut CompoundCommand,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match command {
        CompoundCommand::If(command) => {
            rewrite_stmt_seq_source_texts(&mut command.condition, source, visitor)
                + rewrite_stmt_seq_source_texts(&mut command.then_branch, source, visitor)
                + command
                    .elif_branches
                    .iter_mut()
                    .map(|(condition, body)| {
                        rewrite_stmt_seq_source_texts(condition, source, visitor)
                            + rewrite_stmt_seq_source_texts(body, source, visitor)
                    })
                    .sum::<usize>()
                + command.else_branch.as_mut().map_or(0, |body| {
                    rewrite_stmt_seq_source_texts(body, source, visitor)
                })
        }
        CompoundCommand::For(command) => {
            command
                .words
                .iter_mut()
                .flat_map(|words| words.iter_mut())
                .map(|word| rewrite_word_source_texts(word, source, visitor))
                .sum::<usize>()
                + rewrite_stmt_seq_source_texts(&mut command.body, source, visitor)
        }
        CompoundCommand::Repeat(command) => {
            rewrite_word_source_texts(&mut command.count, source, visitor)
                + rewrite_stmt_seq_source_texts(&mut command.body, source, visitor)
        }
        CompoundCommand::Foreach(command) => {
            command
                .words
                .iter_mut()
                .map(|word| rewrite_word_source_texts(word, source, visitor))
                .sum::<usize>()
                + rewrite_stmt_seq_source_texts(&mut command.body, source, visitor)
        }
        CompoundCommand::ArithmeticFor(command) => {
            rewrite_stmt_seq_source_texts(&mut command.body, source, visitor)
        }
        CompoundCommand::While(command) => {
            rewrite_stmt_seq_source_texts(&mut command.condition, source, visitor)
                + rewrite_stmt_seq_source_texts(&mut command.body, source, visitor)
        }
        CompoundCommand::Until(command) => {
            rewrite_stmt_seq_source_texts(&mut command.condition, source, visitor)
                + rewrite_stmt_seq_source_texts(&mut command.body, source, visitor)
        }
        CompoundCommand::Case(command) => {
            rewrite_word_source_texts(&mut command.word, source, visitor)
                + command
                    .cases
                    .iter_mut()
                    .map(|item| {
                        item.patterns
                            .iter_mut()
                            .map(|pattern| rewrite_pattern_source_texts(pattern, source, visitor))
                            .sum::<usize>()
                            + rewrite_stmt_seq_source_texts(&mut item.body, source, visitor)
                    })
                    .sum::<usize>()
        }
        CompoundCommand::Select(command) => {
            command
                .words
                .iter_mut()
                .map(|word| rewrite_word_source_texts(word, source, visitor))
                .sum::<usize>()
                + rewrite_stmt_seq_source_texts(&mut command.body, source, visitor)
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            rewrite_stmt_seq_source_texts(commands, source, visitor)
        }
        CompoundCommand::Arithmetic(_) => 0,
        CompoundCommand::Time(command) => command.command.as_mut().map_or(0, |command| {
            rewrite_stmt_source_texts(command, source, visitor)
        }),
        CompoundCommand::Conditional(command) => {
            rewrite_conditional_source_texts(command, source, visitor)
        }
        CompoundCommand::Coproc(command) => {
            rewrite_stmt_source_texts(command.body.as_mut(), source, visitor)
        }
        CompoundCommand::Always(command) => {
            rewrite_stmt_seq_source_texts(&mut command.body, source, visitor)
                + rewrite_stmt_seq_source_texts(&mut command.always_body, source, visitor)
        }
    }
}

fn rewrite_stmt_seq_source_texts(
    commands: &mut StmtSeq,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    commands
        .iter_mut()
        .map(|command| rewrite_stmt_source_texts(command, source, visitor))
        .sum()
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
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            rewrite_word_source_texts(word, source, visitor)
        }
        ConditionalExpr::Pattern(pattern) => rewrite_pattern_source_texts(pattern, source, visitor),
        ConditionalExpr::VarRef(reference) => {
            rewrite_var_ref_source_texts(reference, source, visitor)
        }
    }
}

fn rewrite_assignment_source_texts(
    assignment: &mut Assignment,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    let mut count = rewrite_var_ref_source_texts(&mut assignment.target, source, visitor);
    count += match &mut assignment.value {
        AssignmentValue::Scalar(word) => rewrite_word_source_texts(word, source, visitor),
        AssignmentValue::Compound(array) => array
            .elements
            .iter_mut()
            .map(|element| match element {
                ArrayElem::Sequential(word) => rewrite_word_source_texts(word, source, visitor),
                ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                    rewrite_subscript_source_texts(key, source, visitor)
                        + rewrite_word_source_texts(value, source, visitor)
                }
            })
            .sum(),
    };
    count
}

fn rewrite_var_ref_source_texts(
    reference: &mut VarRef,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    reference.subscript.as_mut().map_or(0, |subscript| {
        rewrite_subscript_source_texts(subscript, source, visitor)
    })
}

fn rewrite_subscript_source_texts(
    subscript: &mut shuck_ast::Subscript,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    let mut count = visitor(&mut subscript.text, source);
    if let Some(raw) = &mut subscript.raw {
        count += visitor(raw, source);
    }
    count
}

fn rewrite_pattern_source_texts(
    pattern: &mut Pattern,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    pattern
        .parts
        .iter_mut()
        .map(|part| match &mut part.kind {
            PatternPart::CharClass(text) => visitor(text, source),
            PatternPart::Group { patterns, .. } => patterns
                .iter_mut()
                .map(|pattern| rewrite_pattern_source_texts(pattern, source, visitor))
                .sum(),
            PatternPart::Word(word) => rewrite_word_source_texts(word, source, visitor),
            PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => 0,
        })
        .sum()
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
                + rewrite_heredoc_body_source_texts(&mut heredoc.body, source, visitor)
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

fn rewrite_heredoc_body_source_texts(
    body: &mut HeredocBody,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    let count: usize = body
        .parts
        .iter_mut()
        .map(|part| rewrite_heredoc_body_part_source_texts(part, source, visitor))
        .sum();

    if count > 0 {
        body.source_backed = false;
    }

    count
}

fn rewrite_word_part_source_texts(
    part: &mut WordPartNode,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match &mut part.kind {
        WordPart::Literal(_) | WordPart::Variable(_) => 0,
        WordPart::ZshQualifiedGlob(glob) => {
            glob.segments
                .iter_mut()
                .map(|segment| rewrite_zsh_glob_segment_source_texts(segment, source, visitor))
                .sum::<usize>()
                + glob.qualifiers.as_mut().map_or(0, |group| {
                    rewrite_zsh_glob_qualifier_group_source_texts(group, source, visitor)
                })
        }
        WordPart::SingleQuoted { value, .. } => visitor(value, source),
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter_mut()
            .map(|part| rewrite_word_part_source_texts(part, source, visitor))
            .sum(),
        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => 0,
        WordPart::ArithmeticExpansion { expression, .. } => visitor(expression, source),
        WordPart::Parameter(parameter) => {
            rewrite_parameter_source_texts(parameter, source, visitor)
        }
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            ..
        } => {
            rewrite_var_ref_source_texts(reference, source, visitor)
                + operand
                    .as_mut()
                    .map_or(0, |operand| visitor(operand, source))
                + rewrite_parameter_op_source_texts(operator, source, visitor)
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => {
            rewrite_var_ref_source_texts(reference, source, visitor)
        }
        WordPart::Substring {
            reference,
            offset,
            length,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset,
            length,
            ..
        } => {
            rewrite_var_ref_source_texts(reference, source, visitor)
                + visitor(offset, source)
                + length.as_mut().map_or(0, |length| visitor(length, source))
        }
        WordPart::IndirectExpansion {
            reference,
            operand,
            operator,
            ..
        } => {
            rewrite_var_ref_source_texts(reference, source, visitor)
                + operand
                    .as_mut()
                    .map_or(0, |operand| visitor(operand, source))
                + operator.as_mut().map_or(0, |operator| {
                    rewrite_parameter_op_source_texts(operator, source, visitor)
                })
        }
        WordPart::PrefixMatch { .. } => 0,
    }
}

fn rewrite_heredoc_body_part_source_texts(
    part: &mut HeredocBodyPartNode,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match &mut part.kind {
        HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => 0,
        HeredocBodyPart::CommandSubstitution { .. } => 0,
        HeredocBodyPart::ArithmeticExpansion { expression, .. } => visitor(expression, source),
        HeredocBodyPart::Parameter(parameter) => {
            rewrite_parameter_source_texts(parameter, source, visitor)
        }
    }
}

fn rewrite_zsh_glob_segment_source_texts(
    segment: &mut shuck_ast::ZshGlobSegment,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match segment {
        shuck_ast::ZshGlobSegment::Pattern(pattern) => {
            rewrite_pattern_source_texts(pattern, source, visitor)
        }
        shuck_ast::ZshGlobSegment::InlineControl(_) => 0,
    }
}

fn rewrite_zsh_glob_qualifier_group_source_texts(
    group: &mut shuck_ast::ZshGlobQualifierGroup,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    group
        .fragments
        .iter_mut()
        .map(|fragment| match fragment {
            shuck_ast::ZshGlobQualifier::Negation { .. }
            | shuck_ast::ZshGlobQualifier::Flag { .. } => 0,
            shuck_ast::ZshGlobQualifier::LetterSequence { text, .. } => visitor(text, source),
            shuck_ast::ZshGlobQualifier::NumericArgument { start, end, .. } => {
                visitor(start, source) + end.as_mut().map_or(0, |end| visitor(end, source))
            }
        })
        .sum()
}

fn rewrite_parameter_source_texts(
    parameter: &mut ParameterExpansion,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    let count = match &mut parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                rewrite_var_ref_source_texts(reference, source, visitor)
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand,
                ..
            } => {
                rewrite_var_ref_source_texts(reference, source, visitor)
                    + operand
                        .as_mut()
                        .map_or(0, |operand| visitor(operand, source))
                    + operator.as_mut().map_or(0, |operator| {
                        rewrite_parameter_op_source_texts(operator, source, visitor)
                    })
            }
            BourneParameterExpansion::PrefixMatch { .. } => 0,
            BourneParameterExpansion::Slice {
                reference,
                offset,
                length,
                ..
            } => {
                rewrite_var_ref_source_texts(reference, source, visitor)
                    + visitor(offset, source)
                    + length.as_mut().map_or(0, |length| visitor(length, source))
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                ..
            } => {
                rewrite_var_ref_source_texts(reference, source, visitor)
                    + operand
                        .as_mut()
                        .map_or(0, |operand| visitor(operand, source))
                    + rewrite_parameter_op_source_texts(operator, source, visitor)
            }
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            let mut count = match &mut syntax.target {
                ZshExpansionTarget::Reference(reference) => {
                    rewrite_var_ref_source_texts(reference, source, visitor)
                }
                ZshExpansionTarget::Word(word) => rewrite_word_source_texts(word, source, visitor),
                ZshExpansionTarget::Nested(parameter) => {
                    rewrite_parameter_source_texts(parameter, source, visitor)
                }
                ZshExpansionTarget::Empty => 0,
            };
            if let Some(operation) = &mut syntax.operation {
                count += match operation {
                    ZshExpansionOperation::PatternOperation { operand, .. }
                    | ZshExpansionOperation::Defaulting { operand, .. }
                    | ZshExpansionOperation::TrimOperation { operand, .. } => {
                        let _ = operand;
                        0
                    }
                    ZshExpansionOperation::ReplacementOperation {
                        pattern,
                        replacement,
                        ..
                    } => {
                        let _ = pattern;
                        let _ = replacement;
                        0
                    }
                    ZshExpansionOperation::Slice { offset, length, .. } => {
                        let _ = offset;
                        let _ = length;
                        0
                    }
                    ZshExpansionOperation::Unknown { .. } => 0,
                };
            }
            count
        }
    };

    if count > 0 {
        parameter.raw_body = SourceText::cooked(
            parameter.raw_body.span(),
            render_parameter_raw_body(parameter, source),
        );
    }

    count
}

fn render_parameter_raw_body(parameter: &ParameterExpansion, source: &str) -> String {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => {
            render_bourne_parameter_raw_body(syntax, source)
        }
        ParameterExpansionSyntax::Zsh(syntax) => render_zsh_parameter_raw_body(syntax, source),
    }
}

fn render_bourne_parameter_raw_body(syntax: &BourneParameterExpansion, source: &str) -> String {
    match syntax {
        BourneParameterExpansion::Access { reference } => render_var_ref_syntax(reference, source),
        BourneParameterExpansion::Length { reference } => {
            format!("#{}", render_var_ref_syntax(reference, source))
        }
        BourneParameterExpansion::Indices { reference } => {
            format!("!{}", render_var_ref_syntax(reference, source))
        }
        BourneParameterExpansion::Indirect {
            reference,
            operator,
            operand,
            colon_variant,
            ..
        } => {
            let mut rendered = format!("!{}", render_var_ref_syntax(reference, source));
            if let Some(operator) = operator {
                if *colon_variant {
                    rendered.push(':');
                }
                rendered.push_str(parameter_op_operator_text(operator));
                if let Some(operand) = operand {
                    rendered.push_str(operand.slice(source));
                }
            }
            rendered
        }
        BourneParameterExpansion::PrefixMatch { prefix, kind } => {
            format!("!{}{}", prefix, kind.as_char())
        }
        BourneParameterExpansion::Slice {
            reference,
            offset,
            length,
            ..
        } => {
            let mut rendered = format!(
                "{}:{}",
                render_var_ref_syntax(reference, source),
                offset.slice(source)
            );
            if let Some(length) = length {
                rendered.push(':');
                rendered.push_str(length.slice(source));
            }
            rendered
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            operand,
            colon_variant,
            ..
        } => {
            let mut rendered = render_var_ref_syntax(reference, source);
            match operator {
                ParameterOp::UseDefault
                | ParameterOp::AssignDefault
                | ParameterOp::UseReplacement
                | ParameterOp::Error => {
                    if *colon_variant {
                        rendered.push(':');
                    }
                    rendered.push_str(parameter_op_operator_text(operator));
                    if let Some(operand) = operand {
                        rendered.push_str(operand.slice(source));
                    }
                }
                ParameterOp::RemovePrefixShort { pattern } => {
                    rendered.push('#');
                    pattern.render_syntax_to_buf(source, &mut rendered);
                }
                ParameterOp::RemovePrefixLong { pattern } => {
                    rendered.push_str("##");
                    pattern.render_syntax_to_buf(source, &mut rendered);
                }
                ParameterOp::RemoveSuffixShort { pattern } => {
                    rendered.push('%');
                    pattern.render_syntax_to_buf(source, &mut rendered);
                }
                ParameterOp::RemoveSuffixLong { pattern } => {
                    rendered.push_str("%%");
                    pattern.render_syntax_to_buf(source, &mut rendered);
                }
                ParameterOp::ReplaceFirst {
                    pattern,
                    replacement,
                    ..
                } => {
                    rendered.push('/');
                    pattern.render_syntax_to_buf(source, &mut rendered);
                    rendered.push('/');
                    rendered.push_str(replacement.slice(source));
                }
                ParameterOp::ReplaceAll {
                    pattern,
                    replacement,
                    ..
                } => {
                    rendered.push_str("//");
                    pattern.render_syntax_to_buf(source, &mut rendered);
                    rendered.push('/');
                    rendered.push_str(replacement.slice(source));
                }
                ParameterOp::UpperFirst => rendered.push('^'),
                ParameterOp::UpperAll => rendered.push_str("^^"),
                ParameterOp::LowerFirst => rendered.push(','),
                ParameterOp::LowerAll => rendered.push_str(",,"),
            }
            rendered
        }
        BourneParameterExpansion::Transformation {
            reference,
            operator,
        } => format!("{}@{}", render_var_ref_syntax(reference, source), operator),
    }
}

fn render_zsh_parameter_raw_body(
    syntax: &shuck_ast::ZshParameterExpansion,
    source: &str,
) -> String {
    let mut rendered = String::new();
    let mut modifier_index = 0usize;

    while modifier_index < syntax.modifiers.len() {
        let group_span = syntax.modifiers[modifier_index].span;
        rendered.push('(');
        while modifier_index < syntax.modifiers.len()
            && syntax.modifiers[modifier_index].span == group_span
        {
            let modifier = &syntax.modifiers[modifier_index];
            rendered.push(modifier.name);
            if let Some(delimiter) = modifier.argument_delimiter {
                rendered.push(delimiter);
                if let Some(argument) = &modifier.argument {
                    rendered.push_str(argument.slice(source));
                }
                rendered.push(delimiter);
            }
            modifier_index += 1;
        }
        rendered.push(')');
    }

    match &syntax.target {
        ZshExpansionTarget::Reference(reference) => {
            rendered.push_str(&render_var_ref_syntax(reference, source));
        }
        ZshExpansionTarget::Word(word) => {
            rendered.push_str(&word.render(source));
        }
        ZshExpansionTarget::Nested(parameter) => {
            rendered.push_str("${");
            rendered.push_str(&render_parameter_raw_body(parameter, source));
            rendered.push('}');
        }
        ZshExpansionTarget::Empty => {}
    }

    if let Some(operation) = &syntax.operation {
        match operation {
            ZshExpansionOperation::PatternOperation { operand, .. } => {
                rendered.push_str(":#");
                rendered.push_str(operand.slice(source));
            }
            ZshExpansionOperation::Defaulting {
                kind,
                operand,
                colon_variant,
                ..
            } => {
                if *colon_variant {
                    rendered.push(':');
                }
                rendered.push_str(match kind {
                    shuck_ast::ZshDefaultingOp::UseDefault => "-",
                    shuck_ast::ZshDefaultingOp::AssignDefault => "=",
                    shuck_ast::ZshDefaultingOp::UseReplacement => "+",
                    shuck_ast::ZshDefaultingOp::Error => "?",
                });
                rendered.push_str(operand.slice(source));
            }
            ZshExpansionOperation::TrimOperation { kind, operand, .. } => {
                rendered.push_str(match kind {
                    shuck_ast::ZshTrimOp::RemovePrefixShort => "#",
                    shuck_ast::ZshTrimOp::RemovePrefixLong => "##",
                    shuck_ast::ZshTrimOp::RemoveSuffixShort => "%",
                    shuck_ast::ZshTrimOp::RemoveSuffixLong => "%%",
                });
                rendered.push_str(operand.slice(source));
            }
            ZshExpansionOperation::ReplacementOperation {
                kind,
                pattern,
                replacement,
                ..
            } => {
                rendered.push_str(match kind {
                    shuck_ast::ZshReplacementOp::ReplaceFirst => "/",
                    shuck_ast::ZshReplacementOp::ReplaceAll => "//",
                    shuck_ast::ZshReplacementOp::ReplacePrefix => "/#",
                    shuck_ast::ZshReplacementOp::ReplaceSuffix => "/%",
                });
                rendered.push_str(pattern.slice(source));
                if let Some(replacement) = replacement {
                    rendered.push('/');
                    rendered.push_str(replacement.slice(source));
                }
            }
            ZshExpansionOperation::Slice { offset, length, .. } => {
                rendered.push(':');
                rendered.push_str(offset.slice(source));
                if let Some(length) = length {
                    rendered.push(':');
                    rendered.push_str(length.slice(source));
                }
            }
            ZshExpansionOperation::Unknown { text, .. } => rendered.push_str(text.slice(source)),
        }
    }

    rendered
}

fn render_var_ref_syntax(reference: &VarRef, source: &str) -> String {
    let mut rendered = reference.name.to_string();
    if let Some(subscript) = &reference.subscript {
        rendered.push('[');
        rendered.push_str(subscript.syntax_text(source));
        rendered.push(']');
    }
    rendered
}

fn parameter_op_operator_text(operator: &ParameterOp) -> &'static str {
    match operator {
        ParameterOp::UseDefault => "-",
        ParameterOp::AssignDefault => "=",
        ParameterOp::UseReplacement => "+",
        ParameterOp::Error => "?",
        _ => "",
    }
}

fn rewrite_parameter_op_source_texts(
    operator: &mut ParameterOp,
    source: &str,
    visitor: &mut impl FnMut(&mut SourceText, &str) -> usize,
) -> usize {
    match operator {
        ParameterOp::RemovePrefixShort { pattern }
        | ParameterOp::RemovePrefixLong { pattern }
        | ParameterOp::RemoveSuffixShort { pattern }
        | ParameterOp::RemoveSuffixLong { pattern } => {
            rewrite_pattern_source_texts(pattern, source, visitor)
        }
        ParameterOp::ReplaceFirst {
            pattern,
            replacement,
            ..
        }
        | ParameterOp::ReplaceAll {
            pattern,
            replacement,
            ..
        } => rewrite_pattern_source_texts(pattern, source, visitor) + visitor(replacement, source),
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
        ConditionalExpr::Word(_)
        | ConditionalExpr::Pattern(_)
        | ConditionalExpr::Regex(_)
        | ConditionalExpr::VarRef(_) => 0,
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
                    | ConditionalExpr::VarRef(_)
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
        parts: parts.into(),
        span: word.span,
        brace_syntax: Vec::new(),
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
        )]
        .into(),
        span: word.span,
        brace_syntax: Vec::new(),
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

    use crate::ast_format::flatten_comments;
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
        let parsed = Parser::new(source).parse().unwrap();
        let mut file = parsed.file.clone();
        let rewrite = rewrite_by_name(rewrite);
        let changes = (rewrite.apply)(&mut file, source);
        assert!(changes > 0, "expected rewrite `{rewrite:?}` to apply");

        let options = ShellFormatOptions::default();
        let context = ShellFormatContext::new(
            options.resolve(source, Some(Path::new("test.sh"))),
            source,
            Comments::from_ast(source, &flatten_comments(&parsed.file)),
        );
        let formatted = format!(context, [file.format()]).unwrap();
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
            "echo $((a + b))\n"
        );
    }

    #[test]
    fn arithmetic_var_rewrite_skips_special_parameters() {
        assert_eq!(
            format_with_simplify("echo $(( ${!a} + ${#b} ))\n"),
            "echo $((${!a} + ${#b}))\n"
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
    fn quote_tightening_rewrites_command_substitutions_inside_heredoc_bodies() {
        assert_eq!(
            format_with_simplify("cat <<EOF\n$(printf \"%s\" \"fo\\$o\")\nEOF\n"),
            "cat <<EOF\n$(printf '%s' 'fo$o')\nEOF\n"
        );
    }

    #[test]
    fn arithmetic_var_rewrite_updates_expanding_heredoc_bodies() {
        assert_eq!(
            format_with_simplify("cat <<EOF\n$(( $a + ${b} ))\nEOF\n"),
            "cat <<EOF\n$((a + b))\nEOF\n"
        );
    }

    #[test]
    fn simplify_report_tracks_applied_rewrites() {
        let parsed = Parser::new("echo $(( $a + ${b} ))\n").parse().unwrap();
        let mut file = parsed.file.clone();
        let report = simplify_file(&mut file, "echo $(( $a + ${b} ))\n");

        assert_eq!(report.total_changes(), 1);
        assert_eq!(report.applied().len(), 1);
        assert_eq!(report.applied()[0].name, "arithmetic-vars");
        assert_eq!(report.applied()[0].changes, 1);
    }
}
