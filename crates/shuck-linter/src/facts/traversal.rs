// Private AST traversal used only while building linter facts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct WalkContext {
    loop_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConditionKind {
    If,
    Elif,
    While,
    Until,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CommandTraversalContext {
    walk: WalkContext,
    nested_word_command: bool,
    condition_kind: Option<ConditionKind>,
    in_if_condition: bool,
    in_elif_condition: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CommandWalkOptions {
    descend_nested_word_commands: bool,
}

#[derive(Debug, Clone, Copy)]
struct CommandVisit<'a> {
    stmt: &'a Stmt,
    command: &'a Command,
    redirects: &'a [Redirect],
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
struct ArenaCommandVisit<'a> {
    stmt: StmtView<'a>,
    command: CommandView<'a>,
    redirects: &'a [RedirectNode],
}

fn walk_commands<'a, F>(commands: &'a StmtSeq, options: CommandWalkOptions, visitor: &mut F)
where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    collect_command_visits(
        commands,
        options,
        CommandTraversalContext::default(),
        visitor,
    );
}

fn iter_commands<'a>(
    commands: &'a StmtSeq,
    options: CommandWalkOptions,
) -> impl Iterator<Item = CommandVisit<'a>> {
    let mut visits = Vec::new();
    walk_commands(commands, options, &mut |visit, _| {
        visits.push(visit);
    });
    visits.into_iter()
}

fn walk_arena_commands<'a, F>(
    commands: StmtSeqView<'a>,
    options: CommandWalkOptions,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    collect_arena_command_visits(
        commands,
        options,
        CommandTraversalContext::default(),
        visitor,
    );
}

#[allow(dead_code)]
fn iter_arena_commands<'a>(
    commands: StmtSeqView<'a>,
    options: CommandWalkOptions,
) -> impl Iterator<Item = ArenaCommandVisit<'a>> {
    let mut visits = Vec::new();
    walk_arena_commands(commands, options, &mut |visit, _| {
        visits.push(visit);
    });
    visits.into_iter()
}

fn zsh_glob_patterns(glob: &shuck_ast::ZshQualifiedGlob) -> impl Iterator<Item = &Pattern> + '_ {
    glob.segments.iter().filter_map(|segment| match segment {
        ZshGlobSegment::Pattern(pattern) => Some(pattern),
        ZshGlobSegment::InlineControl(_) => None,
    })
}

fn command_assignments(command: &Command) -> &[Assignment] {
    match command {
        Command::Simple(command) => &command.assignments,
        Command::Builtin(command) => builtin_assignments(command),
        Command::Decl(command) => &command.assignments,
        Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => &[],
    }
}

#[allow(dead_code)]
fn arena_command_assignments(command: CommandView<'_>) -> &[AssignmentNode] {
    match command.kind() {
        ArenaFileCommandKind::Simple => command
            .simple()
            .map(|command| command.assignments())
            .unwrap_or(&[]),
        ArenaFileCommandKind::Builtin => command
            .builtin()
            .map(|command| command.assignments())
            .unwrap_or(&[]),
        ArenaFileCommandKind::Decl => command
            .decl()
            .map(|command| command.assignments())
            .unwrap_or(&[]),
        ArenaFileCommandKind::Binary
        | ArenaFileCommandKind::Compound
        | ArenaFileCommandKind::Function
        | ArenaFileCommandKind::AnonymousFunction => &[],
    }
}

#[allow(dead_code)]
fn arena_declaration_operands(command: CommandView<'_>) -> &[DeclOperandNode] {
    command.decl().map(|command| command.operands()).unwrap_or(&[])
}

fn declaration_operands(command: &Command) -> &[DeclOperand] {
    match command {
        Command::Decl(command) => &command.operands,
        Command::Simple(_)
        | Command::Builtin(_)
        | Command::Binary(_)
        | Command::Compound(_)
        | Command::Function(_)
        | Command::AnonymousFunction(_) => &[],
    }
}

fn visit_arithmetic_words<'a>(
    expression: &'a ArithmeticExprNode,
    visitor: &mut impl FnMut(&'a Word),
) {
    visit_arithmetic_words_in_expr(expression, visitor);
}

fn visit_var_ref_subscript_words_with_source<'a>(
    reference: &'a VarRef,
    _source: &'a str,
    visitor: &mut impl FnMut(&'a Word),
) {
    visit_subscript_words(reference.subscript.as_deref(), _source, visitor);
}

fn visit_subscript_words<'a>(
    subscript: Option<&'a Subscript>,
    _source: &'a str,
    visitor: &mut impl FnMut(&'a Word),
) {
    let Some(subscript) = subscript else {
        return;
    };
    if subscript.selector().is_some() {
        return;
    }
    if let Some(expression) = subscript.arithmetic_ast.as_ref() {
        visit_arithmetic_words_in_expr(expression, visitor);
        return;
    }

    if let Some(word) = subscript.word_ast() {
        visitor(word);
        return;
    }

    debug_assert!(
        subscript.word_ast().is_some(),
        "ordinary subscripts should always carry a word AST"
    );
}

fn visit_arithmetic_words_in_expr<'a>(
    expression: &'a ArithmeticExprNode,
    visitor: &mut impl FnMut(&'a Word),
) {
    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => visit_arithmetic_words_in_expr(index, visitor),
        ArithmeticExpr::ShellWord(word) => visitor(word),
        ArithmeticExpr::Parenthesized { expression } => {
            visit_arithmetic_words_in_expr(expression, visitor)
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            visit_arithmetic_words_in_expr(expr, visitor)
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            visit_arithmetic_words_in_expr(left, visitor);
            visit_arithmetic_words_in_expr(right, visitor);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visit_arithmetic_words_in_expr(condition, visitor);
            visit_arithmetic_words_in_expr(then_expr, visitor);
            visit_arithmetic_words_in_expr(else_expr, visitor);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            visit_arithmetic_lvalue_words(target, visitor);
            visit_arithmetic_words_in_expr(value, visitor);
        }
    }
}

fn collect_optional_arithmetic_words<'a>(
    expression: Option<&'a ArithmeticExprNode>,
    words: &mut Vec<&'a Word>,
) {
    if let Some(expression) = expression {
        collect_arithmetic_words(expression, words);
    }
}

fn collect_var_ref_subscript_words<'a>(reference: &'a VarRef, words: &mut Vec<&'a Word>) {
    collect_optional_arithmetic_words(
        reference
            .subscript
            .as_ref()
            .and_then(|subscript| subscript.arithmetic_ast.as_ref()),
        words,
    );
}

fn collect_arithmetic_lvalue_words<'a>(target: &'a ArithmeticLvalue, words: &mut Vec<&'a Word>) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => collect_arithmetic_words(index, words),
    }
}

fn collect_arithmetic_words<'a>(expression: &'a ArithmeticExprNode, words: &mut Vec<&'a Word>) {
    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => collect_arithmetic_words(index, words),
        ArithmeticExpr::ShellWord(word) => words.push(word),
        ArithmeticExpr::Parenthesized { expression } => collect_arithmetic_words(expression, words),
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            collect_arithmetic_words(expr, words)
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            collect_arithmetic_words(left, words);
            collect_arithmetic_words(right, words);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_arithmetic_words(condition, words);
            collect_arithmetic_words(then_expr, words);
            collect_arithmetic_words(else_expr, words);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            collect_arithmetic_lvalue_words(target, words);
            collect_arithmetic_words(value, words);
        }
    }
}

fn collect_arithmetic_expression_visits<'a, F>(
    expression: &'a ArithmeticExprNode,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    let mut arithmetic_words = Vec::new();
    collect_arithmetic_words(expression, &mut arithmetic_words);
    for word in arithmetic_words {
        collect_word_visits(word, options, context, visitor);
    }
}

fn visit_arithmetic_lvalue_words<'a>(
    target: &'a ArithmeticLvalue,
    visitor: &mut impl FnMut(&'a Word),
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => visit_arithmetic_words_in_expr(index, visitor),
    }
}

fn collect_command_visits<'a, F>(
    commands: &'a StmtSeq,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for stmt in commands.iter() {
        collect_command_visit(stmt, options, context, visitor);
    }
}

fn collect_arena_command_visits<'a, F>(
    commands: StmtSeqView<'a>,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    for stmt in commands.stmts() {
        collect_arena_command_visit(stmt, options, context, visitor);
    }
}

fn collect_arena_command_visit<'a, F>(
    stmt: StmtView<'a>,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    visitor(
        ArenaCommandVisit {
            stmt,
            command: stmt.command(),
            redirects: stmt.redirects(),
        },
        context,
    );

    let command = stmt.command();
    let store = command.store();

    match command.kind() {
        ArenaFileCommandKind::Simple => {
            let command = command.simple().expect("simple command view");
            collect_arena_assignment_visits(
                store,
                command.assignments(),
                options,
                context,
                visitor,
            );
            collect_arena_word_visit(command.name(), options, context, visitor);
            collect_arena_word_ids_visits(store, command.arg_ids(), options, context, visitor);
        }
        ArenaFileCommandKind::Builtin => {
            let command = command.builtin().expect("builtin command view");
            collect_arena_assignment_visits(
                store,
                command.assignments(),
                options,
                context,
                visitor,
            );
            if let Some(word) = command.primary() {
                collect_arena_word_visit(word, options, context, visitor);
            }
            collect_arena_word_ids_visits(store, command.extra_arg_ids(), options, context, visitor);
        }
        ArenaFileCommandKind::Decl => {
            let command = command.decl().expect("decl command view");
            collect_arena_assignment_visits(
                store,
                command.assignments(),
                options,
                context,
                visitor,
            );
            for operand in command.operands() {
                match operand {
                    DeclOperandNode::Flag(word) | DeclOperandNode::Dynamic(word) => {
                        collect_arena_word_visit(store.word(*word), options, context, visitor);
                    }
                    DeclOperandNode::Name(_) => {}
                    DeclOperandNode::Assignment(assignment) => {
                        collect_arena_assignment_visit(store, assignment, options, context, visitor);
                    }
                }
            }
        }
        ArenaFileCommandKind::Binary => {
            let command = command.binary().expect("binary command view");
            collect_arena_command_visits(command.left(), options, context, visitor);
            collect_arena_command_visits(command.right(), options, context, visitor);
        }
        ArenaFileCommandKind::Compound => {
            collect_arena_compound_visits(
                command.compound().expect("compound command view"),
                options,
                context,
                visitor,
            );
        }
        ArenaFileCommandKind::Function => {
            let function = command.function().expect("function command view");
            for entry in function.entries() {
                collect_arena_word_visit(store.word(entry.word), options, context, visitor);
            }
            collect_arena_command_visits(function.body(), options, context, visitor);
        }
        ArenaFileCommandKind::AnonymousFunction => {
            let function = command
                .anonymous_function()
                .expect("anonymous function command view");
            collect_arena_word_ids_visits(store, function.arg_ids(), options, context, visitor);
            collect_arena_command_visits(function.body(), options, context, visitor);
        }
    }

    collect_arena_redirect_visits(stmt, options, context, visitor);
}

fn collect_arena_compound_visits<'a, F>(
    command: shuck_ast::CompoundCommandView<'a>,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    let store = command.store();
    match command.node() {
        CompoundCommandNode::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
            ..
        } => {
            collect_arena_command_visits(
                store.stmt_seq(*condition),
                options,
                condition_context(context, ConditionKind::If),
                visitor,
            );
            collect_arena_command_visits(store.stmt_seq(*then_branch), options, context, visitor);
            for branch in store.elif_branches(*elif_branches) {
                collect_arena_command_visits(
                    store.stmt_seq(branch.condition),
                    options,
                    condition_context(context, ConditionKind::Elif),
                    visitor,
                );
                collect_arena_command_visits(store.stmt_seq(branch.body), options, context, visitor);
            }
            if let Some(body) = else_branch {
                collect_arena_command_visits(store.stmt_seq(*body), options, context, visitor);
            }
        }
        CompoundCommandNode::For { words, body, .. } => {
            if let Some(words) = words {
                collect_arena_word_ids_visits(
                    store,
                    store.word_ids(*words),
                    options,
                    context,
                    visitor,
                );
            }
            collect_arena_command_visits(
                store.stmt_seq(*body),
                options,
                loop_context(context),
                visitor,
            );
        }
        CompoundCommandNode::Repeat { count, body, .. } => {
            collect_arena_word_visit(store.word(*count), options, context, visitor);
            collect_arena_command_visits(
                store.stmt_seq(*body),
                options,
                loop_context(context),
                visitor,
            );
        }
        CompoundCommandNode::Foreach { words, body, .. } => {
            collect_arena_word_ids_visits(store, store.word_ids(*words), options, context, visitor);
            collect_arena_command_visits(
                store.stmt_seq(*body),
                options,
                loop_context(context),
                visitor,
            );
        }
        CompoundCommandNode::ArithmeticFor(command) => {
            for expression in [
                command.init_ast.as_ref(),
                command.condition_ast.as_ref(),
                command.step_ast.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                collect_arena_arithmetic_expression_visits(expression, options, context, visitor);
            }
            collect_arena_command_visits(
                store.stmt_seq(command.body),
                options,
                loop_context(context),
                visitor,
            );
        }
        CompoundCommandNode::While { condition, body } => {
            let loop_context = loop_context(context);
            collect_arena_command_visits(
                store.stmt_seq(*condition),
                options,
                condition_context(loop_context, ConditionKind::While),
                visitor,
            );
            collect_arena_command_visits(store.stmt_seq(*body), options, loop_context, visitor);
        }
        CompoundCommandNode::Until { condition, body } => {
            let loop_context = loop_context(context);
            collect_arena_command_visits(
                store.stmt_seq(*condition),
                options,
                condition_context(loop_context, ConditionKind::Until),
                visitor,
            );
            collect_arena_command_visits(store.stmt_seq(*body), options, loop_context, visitor);
        }
        CompoundCommandNode::Case { word, cases } => {
            collect_arena_word_visit(store.word(*word), options, context, visitor);
            for case in store.case_items(*cases) {
                collect_arena_pattern_slice_visits(
                    store,
                    store.patterns(case.patterns),
                    options,
                    context,
                    visitor,
                );
                collect_arena_command_visits(store.stmt_seq(case.body), options, context, visitor);
            }
        }
        CompoundCommandNode::Select { words, body, .. } => {
            collect_arena_word_ids_visits(store, store.word_ids(*words), options, context, visitor);
            collect_arena_command_visits(
                store.stmt_seq(*body),
                options,
                loop_context(context),
                visitor,
            );
        }
        CompoundCommandNode::Subshell(body) | CompoundCommandNode::BraceGroup(body) => {
            collect_arena_command_visits(store.stmt_seq(*body), options, context, visitor);
        }
        CompoundCommandNode::Always { body, always_body } => {
            collect_arena_command_visits(store.stmt_seq(*body), options, context, visitor);
            collect_arena_command_visits(store.stmt_seq(*always_body), options, context, visitor);
        }
        CompoundCommandNode::Arithmetic(command) => {
            if let Some(expression) = command.expr_ast.as_ref() {
                collect_arena_arithmetic_expression_visits(expression, options, context, visitor);
            }
        }
        CompoundCommandNode::Time { command, .. } => {
            if let Some(command) = command {
                collect_arena_command_visits(store.stmt_seq(*command), options, context, visitor);
            }
        }
        CompoundCommandNode::Conditional(command) => {
            collect_arena_conditional_visits(&command.expression, options, context, visitor);
        }
        CompoundCommandNode::Coproc { body, .. } => {
            collect_arena_command_visits(store.stmt_seq(*body), options, context, visitor);
        }
    }
}

fn collect_arena_assignment_visits<'a, F>(
    store: &'a AstStore,
    assignments: &'a [AssignmentNode],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    for assignment in assignments {
        collect_arena_assignment_visit(store, assignment, options, context, visitor);
    }
}

fn collect_arena_assignment_visit<'a, F>(
    store: &'a AstStore,
    assignment: &'a AssignmentNode,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    match &assignment.value {
        AssignmentValueNode::Scalar(word) => {
            collect_arena_word_visit(store.word(*word), options, context, visitor);
        }
        AssignmentValueNode::Compound(array) => {
            for element in store.array_elems(array.elements) {
                match element {
                    ArrayElemNode::Sequential(value) => {
                        collect_arena_word_visit(store.word(value.word), options, context, visitor);
                    }
                    ArrayElemNode::Keyed { key, value }
                    | ArrayElemNode::KeyedAppend { key, value } => {
                        collect_arena_subscript_visit(key, store, options, context, visitor);
                        collect_arena_word_visit(store.word(value.word), options, context, visitor);
                    }
                }
            }
        }
    }
}

fn collect_arena_redirect_visits<'a, F>(
    stmt: StmtView<'a>,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    let store = stmt.command().store();
    for redirect in stmt.redirects() {
        match &redirect.target {
            RedirectTargetNode::Word(word) => {
                collect_arena_word_visit(store.word(*word), options, context, visitor);
            }
            RedirectTargetNode::Heredoc(heredoc) => {
                collect_arena_word_visit(store.word(heredoc.delimiter.raw), options, context, visitor);
                if heredoc.delimiter.expands_body {
                    collect_arena_heredoc_body_visits(
                        store,
                        store.heredoc_body_parts(heredoc.body.parts),
                        options,
                        context,
                        visitor,
                    );
                }
            }
        }
    }
}

fn collect_arena_word_ids_visits<'a, F>(
    store: &'a AstStore,
    words: &[WordId],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    for word in words {
        collect_arena_word_visit(store.word(*word), options, context, visitor);
    }
}

fn collect_arena_word_visit<'a, F>(
    word: shuck_ast::WordView<'a>,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    if !options.descend_nested_word_commands {
        return;
    }

    collect_arena_word_part_visits(
        word.parts(),
        word.store(),
        options,
        nested_word_context(context),
        visitor,
    );
}

fn collect_arena_word_part_visits<'a, F>(
    parts: &'a [WordPartArenaNode],
    store: &'a AstStore,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    for part in parts {
        match &part.kind {
            WordPartArena::ZshQualifiedGlob(glob) => {
                for segment in store.zsh_glob_segments(glob.segments) {
                    if let ZshGlobSegmentNode::Pattern(pattern) = segment {
                        collect_arena_pattern_visits(store, pattern, options, context, visitor);
                    }
                }
            }
            WordPartArena::DoubleQuoted { parts, .. } => {
                collect_arena_word_part_visits(
                    store.word_parts(*parts),
                    store,
                    options,
                    context,
                    visitor,
                );
            }
            WordPartArena::CommandSubstitution { body, .. }
            | WordPartArena::ProcessSubstitution { body, .. } => {
                collect_arena_command_visits(store.stmt_seq(*body), options, context, visitor);
            }
            WordPartArena::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                if let Some(expression) = expression_ast.as_ref() {
                    collect_arena_arithmetic_expression_visits(
                        expression, options, context, visitor,
                    );
                } else {
                    collect_arena_word_visit(
                        store.word(*expression_word_ast),
                        options,
                        context,
                        visitor,
                    );
                }
            }
            WordPartArena::Parameter(parameter) => {
                collect_arena_parameter_expansion_visits(
                    store, parameter, options, context, visitor,
                );
            }
            WordPartArena::ParameterExpansion {
                reference,
                operand_word_ast,
                ..
            }
            | WordPartArena::IndirectExpansion {
                reference,
                operand_word_ast,
                ..
            } => {
                collect_arena_var_ref_word_visits(reference, store, options, context, visitor);
                if let Some(word) = operand_word_ast {
                    collect_arena_word_visit(store.word(*word), options, context, visitor);
                }
            }
            WordPartArena::Length(reference)
            | WordPartArena::ArrayAccess(reference)
            | WordPartArena::ArrayLength(reference)
            | WordPartArena::ArrayIndices(reference)
            | WordPartArena::Transformation { reference, .. } => {
                collect_arena_var_ref_word_visits(reference, store, options, context, visitor);
            }
            WordPartArena::Substring {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPartArena::ArraySlice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_arena_var_ref_word_visits(reference, store, options, context, visitor);
                if let Some(expression) = offset_ast.as_ref() {
                    collect_arena_arithmetic_expression_visits(
                        expression, options, context, visitor,
                    );
                } else {
                    collect_arena_word_visit(store.word(*offset_word_ast), options, context, visitor);
                }
                if let Some(expression) = length_ast.as_ref() {
                    collect_arena_arithmetic_expression_visits(
                        expression, options, context, visitor,
                    );
                } else if let Some(word) = length_word_ast {
                    collect_arena_word_visit(store.word(*word), options, context, visitor);
                }
            }
            WordPartArena::Literal(_)
            | WordPartArena::SingleQuoted { .. }
            | WordPartArena::Variable(_)
            | WordPartArena::PrefixMatch { .. } => {}
        }
    }
}

fn collect_arena_heredoc_body_visits<'a, F>(
    store: &'a AstStore,
    parts: &'a [shuck_ast::ArenaHeredocBodyPartNode],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    for part in parts {
        match &part.kind {
            ArenaHeredocBodyPart::CommandSubstitution { body, .. } => {
                collect_arena_command_visits(
                    store.stmt_seq(*body),
                    options,
                    nested_word_context(context),
                    visitor,
                );
            }
            ArenaHeredocBodyPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                if let Some(expression) = expression_ast.as_ref() {
                    collect_arena_arithmetic_expression_visits(
                        expression,
                        options,
                        nested_word_context(context),
                        visitor,
                    );
                } else {
                    collect_arena_word_visit(
                        store.word(*expression_word_ast),
                        options,
                        context,
                        visitor,
                    );
                }
            }
            ArenaHeredocBodyPart::Parameter(parameter) => {
                collect_arena_parameter_expansion_visits(
                    store,
                    parameter,
                    options,
                    nested_word_context(context),
                    visitor,
                );
            }
            ArenaHeredocBodyPart::Literal(_) | ArenaHeredocBodyPart::Variable(_) => {}
        }
    }
}

fn collect_arena_subscript_visit<'a, F>(
    subscript: &'a SubscriptNode,
    store: &'a AstStore,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    if let Some(expression) = subscript.arithmetic_ast.as_ref() {
        collect_arena_arithmetic_expression_visits(expression, options, context, visitor);
    } else if let Some(word) = subscript.word_ast {
        collect_arena_word_visit(store.word(word), options, context, visitor);
    }
}

fn collect_arena_var_ref_word_visits<'a, F>(
    reference: &'a VarRefNode,
    store: &'a AstStore,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    if let Some(subscript) = reference.subscript.as_deref() {
        collect_arena_subscript_visit(subscript, store, options, context, visitor);
    }
}

fn collect_arena_parameter_expansion_visits<'a, F>(
    store: &'a AstStore,
    parameter: &'a ParameterExpansionNode,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    match &parameter.syntax {
        ParameterExpansionSyntaxNode::Bourne(syntax) => match syntax {
            BourneParameterExpansionNode::Access { reference }
            | BourneParameterExpansionNode::Length { reference }
            | BourneParameterExpansionNode::Indices { reference }
            | BourneParameterExpansionNode::Transformation { reference, .. } => {
                collect_arena_var_ref_word_visits(reference, store, options, context, visitor);
            }
            BourneParameterExpansionNode::Indirect {
                reference,
                operand_word_ast,
                ..
            }
            | BourneParameterExpansionNode::Operation {
                reference,
                operand_word_ast,
                ..
            } => {
                collect_arena_var_ref_word_visits(reference, store, options, context, visitor);
                if let Some(word) = operand_word_ast {
                    collect_arena_word_visit(store.word(*word), options, context, visitor);
                }
            }
            BourneParameterExpansionNode::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_arena_var_ref_word_visits(reference, store, options, context, visitor);
                if let Some(expression) = offset_ast.as_ref() {
                    collect_arena_arithmetic_expression_visits(
                        expression, options, context, visitor,
                    );
                } else {
                    collect_arena_word_visit(store.word(*offset_word_ast), options, context, visitor);
                }
                if let Some(expression) = length_ast.as_ref() {
                    collect_arena_arithmetic_expression_visits(
                        expression, options, context, visitor,
                    );
                } else if let Some(word) = length_word_ast {
                    collect_arena_word_visit(store.word(*word), options, context, visitor);
                }
            }
            BourneParameterExpansionNode::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntaxNode::Zsh(syntax) => {
            collect_arena_zsh_target_visits(&syntax.target, store, options, context, visitor);
            for modifier in store.zsh_modifiers(syntax.modifiers) {
                if let Some(word) = modifier.argument_word_ast {
                    collect_arena_word_visit(store.word(word), options, context, visitor);
                }
            }
            if let Some(operation) = &syntax.operation {
                match operation {
                    ZshExpansionOperationNode::PatternOperation {
                        operand_word_ast, ..
                    }
                    | ZshExpansionOperationNode::Defaulting {
                        operand_word_ast, ..
                    }
                    | ZshExpansionOperationNode::TrimOperation {
                        operand_word_ast, ..
                    } => {
                        collect_arena_word_visit(
                            store.word(*operand_word_ast),
                            options,
                            context,
                            visitor,
                        );
                    }
                    ZshExpansionOperationNode::ReplacementOperation {
                        pattern_word_ast,
                        replacement_word_ast,
                        ..
                    } => {
                        collect_arena_word_visit(
                            store.word(*pattern_word_ast),
                            options,
                            context,
                            visitor,
                        );
                        if let Some(word) = replacement_word_ast {
                            collect_arena_word_visit(store.word(*word), options, context, visitor);
                        }
                    }
                    ZshExpansionOperationNode::Slice {
                        offset_word_ast,
                        length_word_ast,
                        ..
                    } => {
                        collect_arena_word_visit(
                            store.word(*offset_word_ast),
                            options,
                            context,
                            visitor,
                        );
                        if let Some(word) = length_word_ast {
                            collect_arena_word_visit(store.word(*word), options, context, visitor);
                        }
                    }
                    ZshExpansionOperationNode::Unknown { word_ast, .. } => {
                        collect_arena_word_visit(store.word(*word_ast), options, context, visitor);
                    }
                }
            }
        }
    }
}

fn collect_arena_zsh_target_visits<'a, F>(
    target: &'a ZshExpansionTargetNode,
    store: &'a AstStore,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    match target {
        ZshExpansionTargetNode::Reference(reference) => {
            collect_arena_var_ref_word_visits(reference, store, options, context, visitor);
        }
        ZshExpansionTargetNode::Nested(parameter) => {
            collect_arena_parameter_expansion_visits(store, parameter, options, context, visitor);
        }
        ZshExpansionTargetNode::Word(word) => {
            collect_arena_word_visit(store.word(*word), options, context, visitor);
        }
        ZshExpansionTargetNode::Empty => {}
    }
}

fn collect_arena_arithmetic_expression_visits<'a, F>(
    expression: &'a ArithmeticExprArenaNode,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    match &expression.kind {
        ArithmeticExprArena::Number(_) | ArithmeticExprArena::Variable(_) => {}
        ArithmeticExprArena::ShellWord(word) => {
            // The expression node does not carry a store, so callers should prefer
            // the word-backed fallback when they need nested command traversal.
            let _ = word;
        }
        ArithmeticExprArena::Indexed { index, .. }
        | ArithmeticExprArena::Parenthesized { expression: index }
        | ArithmeticExprArena::Unary { expr: index, .. }
        | ArithmeticExprArena::Postfix { expr: index, .. } => {
            collect_arena_arithmetic_expression_visits(index, options, context, visitor);
        }
        ArithmeticExprArena::Binary { left, right, .. } => {
            collect_arena_arithmetic_expression_visits(left, options, context, visitor);
            collect_arena_arithmetic_expression_visits(right, options, context, visitor);
        }
        ArithmeticExprArena::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_arena_arithmetic_expression_visits(condition, options, context, visitor);
            collect_arena_arithmetic_expression_visits(then_expr, options, context, visitor);
            collect_arena_arithmetic_expression_visits(else_expr, options, context, visitor);
        }
        ArithmeticExprArena::Assignment { target, value, .. } => {
            if let ArithmeticLvalueArena::Indexed { index, .. } = target {
                collect_arena_arithmetic_expression_visits(index, options, context, visitor);
            }
            collect_arena_arithmetic_expression_visits(value, options, context, visitor);
        }
    }
}

fn collect_arena_conditional_visits<'a, F>(
    expression: &'a ConditionalExprArena,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    match expression {
        ConditionalExprArena::Binary { left, right, .. } => {
            collect_arena_conditional_visits(left, options, context, visitor);
            collect_arena_conditional_visits(right, options, context, visitor);
        }
        ConditionalExprArena::Unary { expr, .. }
        | ConditionalExprArena::Parenthesized { expr, .. } => {
            collect_arena_conditional_visits(expr, options, context, visitor);
        }
        ConditionalExprArena::Pattern(pattern) => {
            // Pattern nodes may contain word-backed fragments, but the expression
            // node does not carry a store. Those fragments are also part of the
            // enclosing command's word list and will be traversed from there.
            let _ = pattern;
        }
        ConditionalExprArena::Word(_)
        | ConditionalExprArena::Regex(_)
        | ConditionalExprArena::VarRef(_) => {}
    }
}

fn collect_arena_pattern_slice_visits<'a, F>(
    store: &'a AstStore,
    patterns: &'a [PatternNode],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    for pattern in patterns {
        collect_arena_pattern_visits(store, pattern, options, context, visitor);
    }
}

fn collect_arena_pattern_visits<'a, F>(
    store: &'a AstStore,
    pattern: &'a PatternNode,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(ArenaCommandVisit<'a>, CommandTraversalContext),
{
    for part in store.pattern_parts(pattern.parts) {
        match &part.kind {
            PatternPartArena::Group { patterns, .. } => {
                collect_arena_pattern_slice_visits(
                    store,
                    store.patterns(*patterns),
                    options,
                    context,
                    visitor,
                );
            }
            PatternPartArena::Word(word) => {
                collect_arena_word_visit(store.word(*word), options, context, visitor);
            }
            PatternPartArena::Literal(_)
            | PatternPartArena::AnyString
            | PatternPartArena::AnyChar
            | PatternPartArena::CharClass(_) => {}
        }
    }
}

fn collect_command_visit<'a, F>(
    stmt: &'a Stmt,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    visitor(
        CommandVisit {
            stmt,
            command: &stmt.command,
            redirects: &stmt.redirects,
        },
        context,
    );

    match &stmt.command {
        Command::Simple(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            collect_word_visits(&command.name, options, context, visitor);
            collect_word_slice_visits(&command.args, options, context, visitor);
        }
        Command::Builtin(command) => collect_builtin_visits(command, options, context, visitor),
        Command::Decl(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_word_visits(word, options, context, visitor);
                    }
                    DeclOperand::Name(_) => {}
                    DeclOperand::Assignment(assignment) => {
                        collect_assignment_visit(assignment, options, context, visitor);
                    }
                }
            }
        }
        Command::Binary(command) => {
            collect_command_visit(&command.left, options, context, visitor);
            collect_command_visit(&command.right, options, context, visitor);
        }
        Command::Compound(command) => {
            collect_compound_visits(command, options, context, visitor);
        }
        Command::Function(FunctionDef { header, body, .. }) => {
            for entry in &header.entries {
                collect_word_visits(&entry.word, options, context, visitor);
            }
            collect_command_visit(body, options, context, visitor);
        }
        Command::AnonymousFunction(function) => {
            collect_word_slice_visits(&function.args, options, context, visitor);
            collect_command_visit(&function.body, options, context, visitor);
        }
    }

    collect_redirect_visits(&stmt.redirects, options, context, visitor);
}

fn condition_context(
    context: CommandTraversalContext,
    kind: ConditionKind,
) -> CommandTraversalContext {
    let (in_if_condition, in_elif_condition) = match kind {
        ConditionKind::If => (true, context.in_elif_condition),
        ConditionKind::Elif => (true, true),
        ConditionKind::While | ConditionKind::Until => {
            (context.in_if_condition, context.in_elif_condition)
        }
    };

    CommandTraversalContext {
        condition_kind: Some(kind),
        in_if_condition,
        in_elif_condition,
        ..context
    }
}

fn loop_context(context: CommandTraversalContext) -> CommandTraversalContext {
    CommandTraversalContext {
        walk: WalkContext {
            loop_depth: context.walk.loop_depth + 1,
        },
        ..context
    }
}

fn nested_word_context(context: CommandTraversalContext) -> CommandTraversalContext {
    CommandTraversalContext {
        nested_word_command: true,
        ..context
    }
}

fn collect_builtin_visits<'a, F>(
    command: &'a BuiltinCommand,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match command {
        BuiltinCommand::Break(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            if let Some(word) = &command.depth {
                collect_word_visits(word, options, context, visitor);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visitor);
        }
        BuiltinCommand::Continue(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            if let Some(word) = &command.depth {
                collect_word_visits(word, options, context, visitor);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visitor);
        }
        BuiltinCommand::Return(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            if let Some(word) = &command.code {
                collect_word_visits(word, options, context, visitor);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visitor);
        }
        BuiltinCommand::Exit(command) => {
            collect_assignment_visits(&command.assignments, options, context, visitor);
            if let Some(word) = &command.code {
                collect_word_visits(word, options, context, visitor);
            }
            collect_word_slice_visits(&command.extra_args, options, context, visitor);
        }
    }
}

fn collect_compound_visits<'a, F>(
    command: &'a CompoundCommand,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match command {
        CompoundCommand::If(command) => {
            collect_command_visits(
                &command.condition,
                options,
                condition_context(context, ConditionKind::If),
                visitor,
            );
            collect_command_visits(&command.then_branch, options, context, visitor);
            for (condition, body) in &command.elif_branches {
                collect_command_visits(
                    condition,
                    options,
                    condition_context(context, ConditionKind::Elif),
                    visitor,
                );
                collect_command_visits(body, options, context, visitor);
            }
            if let Some(body) = &command.else_branch {
                collect_command_visits(body, options, context, visitor);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                collect_word_slice_visits(words, options, context, visitor);
            }
            collect_command_visits(&command.body, options, loop_context(context), visitor);
        }
        CompoundCommand::Repeat(command) => {
            collect_word_visits(&command.count, options, context, visitor);
            collect_command_visits(&command.body, options, loop_context(context), visitor);
        }
        CompoundCommand::Foreach(command) => {
            collect_word_slice_visits(&command.words, options, context, visitor);
            collect_command_visits(&command.body, options, loop_context(context), visitor);
        }
        CompoundCommand::ArithmeticFor(command) => {
            for expression in [
                command.init_ast.as_ref(),
                command.condition_ast.as_ref(),
                command.step_ast.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                collect_arithmetic_expression_visits(expression, options, context, visitor);
            }
            collect_command_visits(&command.body, options, loop_context(context), visitor);
        }
        CompoundCommand::While(command) => {
            let loop_context = loop_context(context);
            collect_command_visits(
                &command.condition,
                options,
                condition_context(loop_context, ConditionKind::While),
                visitor,
            );
            collect_command_visits(&command.body, options, loop_context, visitor);
        }
        CompoundCommand::Until(command) => {
            let loop_context = loop_context(context);
            collect_command_visits(
                &command.condition,
                options,
                condition_context(loop_context, ConditionKind::Until),
                visitor,
            );
            collect_command_visits(&command.body, options, loop_context, visitor);
        }
        CompoundCommand::Case(command) => {
            collect_word_visits(&command.word, options, context, visitor);
            for case in &command.cases {
                collect_pattern_slice_visits(&case.patterns, options, context, visitor);
                collect_command_visits(&case.body, options, context, visitor);
            }
        }
        CompoundCommand::Select(command) => {
            collect_word_slice_visits(&command.words, options, context, visitor);
            collect_command_visits(&command.body, options, loop_context(context), visitor);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            collect_command_visits(commands, options, context, visitor);
        }
        CompoundCommand::Always(command) => {
            collect_command_visits(&command.body, options, context, visitor);
            collect_command_visits(&command.always_body, options, context, visitor);
        }
        CompoundCommand::Arithmetic(command) => {
            if let Some(expression) = command.expr_ast.as_ref() {
                collect_arithmetic_expression_visits(expression, options, context, visitor);
            }
        }
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                collect_command_visit(command, options, context, visitor);
            }
        }
        CompoundCommand::Conditional(command) => {
            collect_conditional_visits(&command.expression, options, context, visitor);
        }
        CompoundCommand::Coproc(command) => {
            collect_command_visit(&command.body, options, context, visitor);
        }
    }
}

fn collect_assignment_visits<'a, F>(
    assignments: &'a [Assignment],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for assignment in assignments {
        collect_assignment_visit(assignment, options, context, visitor);
    }
}

fn collect_assignment_visit<'a, F>(
    assignment: &'a Assignment,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_word_visits(word, options, context, visitor),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => {
                        collect_word_visits(word, options, context, visitor);
                    }
                    ArrayElem::Keyed { value, .. } | ArrayElem::KeyedAppend { value, .. } => {
                        collect_word_visits(value, options, context, visitor);
                    }
                }
            }
        }
    }
}

fn collect_word_slice_visits<'a, F>(
    words: &'a [Word],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for word in words {
        collect_word_visits(word, options, context, visitor);
    }
}

fn collect_pattern_slice_visits<'a, F>(
    patterns: &'a [Pattern],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for pattern in patterns {
        collect_pattern_visits(pattern, options, context, visitor);
    }
}

fn collect_word_visits<'a, F>(
    word: &'a Word,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    if !options.descend_nested_word_commands {
        return;
    }

    collect_word_part_visits(&word.parts, options, nested_word_context(context), visitor);
}

fn collect_word_part_visits<'a, F>(
    parts: &'a [WordPartNode],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for part in parts {
        match &part.kind {
            WordPart::ZshQualifiedGlob(glob) => {
                for pattern in zsh_glob_patterns(glob) {
                    collect_pattern_visits(pattern, options, context, visitor);
                }
            }
            WordPart::DoubleQuoted { parts, .. } => {
                collect_word_part_visits(parts, options, context, visitor);
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    collect_arithmetic_expression_visits(expression_ast, options, context, visitor);
                }
            }
            WordPart::Parameter(parameter) => {
                collect_parameter_expansion_visits(parameter, options, context, visitor);
            }
            WordPart::ParameterExpansion {
                reference,
                operand_word_ast,
                ..
            }
            | WordPart::IndirectExpansion {
                reference,
                operand_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
                if let Some(word) = operand_word_ast.as_ref() {
                    collect_word_visits(word, options, context, visitor);
                }
            }
            WordPart::Length(reference)
            | WordPart::ArrayAccess(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference)
            | WordPart::Transformation { reference, .. } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
            }
            WordPart::Substring {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
                if let Some(expression) = offset_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                } else {
                    collect_word_visits(offset_word_ast, options, context, visitor);
                }
                if let Some(expression) = length_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                } else if let Some(word) = length_word_ast.as_ref() {
                    collect_word_visits(word, options, context, visitor);
                }
            }
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => {
                collect_command_visits(body, options, context, visitor);
            }
            WordPart::Literal(_)
            | WordPart::SingleQuoted { .. }
            | WordPart::Variable(_)
            | WordPart::PrefixMatch { .. } => {}
        }
    }
}

fn collect_var_ref_word_visits<'a, F>(
    reference: &'a VarRef,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    let mut words = Vec::new();
    collect_var_ref_subscript_words(reference, &mut words);
    for word in words {
        collect_word_visits(word, options, context, visitor);
    }
}

fn collect_parameter_expansion_visits<'a, F>(
    parameter: &'a ParameterExpansion,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
                if let Some(word) = operand_word_ast.as_ref() {
                    collect_word_visits(word, options, context, visitor);
                }
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
                if let Some(word) = operand_word_ast.as_ref() {
                    collect_word_visits(word, options, context, visitor);
                }
                if let Some(word) = operator.replacement_word_ast() {
                    collect_word_visits(word, options, context, visitor);
                }
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                offset_word_ast,
                length_ast,
                length_word_ast,
                ..
            } => {
                collect_var_ref_word_visits(reference, options, context, visitor);
                if let Some(expression) = offset_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                } else {
                    collect_word_visits(offset_word_ast, options, context, visitor);
                }
                if let Some(expression) = length_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                } else if let Some(word) = length_word_ast.as_ref() {
                    collect_word_visits(word, options, context, visitor);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            collect_zsh_target_visits(&syntax.target, options, context, visitor);

            for modifier in &syntax.modifiers {
                if let Some(word) = modifier.argument_word_ast() {
                    collect_word_visits(word, options, context, visitor);
                }
            }

            if let Some(operation) = &syntax.operation {
                match operation {
                    shuck_ast::ZshExpansionOperation::PatternOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Defaulting { .. }
                    | shuck_ast::ZshExpansionOperation::TrimOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Unknown { .. } => {
                        if let Some(word) = operation.operand_word_ast() {
                            collect_word_visits(word, options, context, visitor);
                        }
                    }
                    shuck_ast::ZshExpansionOperation::ReplacementOperation { .. } => {
                        if let Some(word) = operation.pattern_word_ast() {
                            collect_word_visits(word, options, context, visitor);
                        }
                        if let Some(word) = operation.replacement_word_ast() {
                            collect_word_visits(word, options, context, visitor);
                        }
                    }
                    shuck_ast::ZshExpansionOperation::Slice { .. } => {
                        if let Some(word) = operation.offset_word_ast() {
                            collect_word_visits(word, options, context, visitor);
                        }
                        if let Some(word) = operation.length_word_ast() {
                            collect_word_visits(word, options, context, visitor);
                        }
                    }
                }
            }
        }
    }
}

fn collect_zsh_target_visits<'a, F>(
    target: &'a ZshExpansionTarget,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match target {
        ZshExpansionTarget::Reference(reference) => {
            collect_var_ref_word_visits(reference, options, context, visitor);
        }
        ZshExpansionTarget::Nested(parameter) => {
            collect_parameter_expansion_visits(parameter, options, context, visitor);
        }
        ZshExpansionTarget::Word(word) => {
            collect_word_visits(word, options, context, visitor);
        }
        ZshExpansionTarget::Empty => {}
    }
}

fn collect_pattern_visits<'a, F>(
    pattern: &'a Pattern,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                collect_pattern_slice_visits(patterns, options, context, visitor);
            }
            PatternPart::Word(word) => collect_word_visits(word, options, context, visitor),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

#[cfg(test)]
mod arena_traversal_tests {
    use shuck_parser::parser::Parser;

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct SeenCommand {
        start: usize,
        end: usize,
        nested_word_command: bool,
        loop_depth: usize,
        in_if_condition: bool,
        in_elif_condition: bool,
        condition_kind: Option<ConditionKind>,
    }

    fn seen_from_context(span: Span, context: CommandTraversalContext) -> SeenCommand {
        SeenCommand {
            start: span.start.offset,
            end: span.end.offset,
            nested_word_command: context.nested_word_command,
            loop_depth: context.walk.loop_depth,
            in_if_condition: context.in_if_condition,
            in_elif_condition: context.in_elif_condition,
            condition_kind: context.condition_kind,
        }
    }

    fn recursive_visits(source: &str) -> Vec<SeenCommand> {
        let output = Parser::new(source).parse().expect("parse");
        let mut visits = Vec::new();
        walk_commands(
            &output.file.body,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
            &mut |visit, context| {
                visits.push(seen_from_context(command_span(visit.command), context));
            },
        );
        visits
    }

    fn arena_visits(source: &str) -> Vec<SeenCommand> {
        let output = Parser::new(source).parse().expect("parse");
        let mut visits = Vec::new();
        walk_arena_commands(
            output.arena_file.view().body(),
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
            &mut |visit, context| {
                visits.push(seen_from_context(visit.command.span(), context));
            },
        );
        visits
    }

    #[test]
    fn arena_command_walk_matches_recursive_contexts() {
        let source = r#"
if test "$x"; then
  echo "$(date)"
elif grep foo file; then
  while read line; do printf '%s\n' "$line"; done
else
  find . -exec sh -c 'echo "$1"' sh {} \;
fi
for name in "$(printf '%s\n' a)"; do :; done
case "$x" in
  a$(echo b)) echo c ;;
esac
"#;

        assert_eq!(arena_visits(source), recursive_visits(source));
    }
}

fn collect_redirect_visits<'a, F>(
    redirects: &'a [Redirect],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    for redirect in redirects {
        if let Some(word) = redirect.word_target() {
            collect_word_visits(word, options, context, visitor);
        } else if let Some(heredoc) = redirect.heredoc()
            && heredoc.delimiter.expands_body
        {
            collect_heredoc_body_part_visits(&heredoc.body.parts, options, context, visitor);
        }
    }
}

fn collect_conditional_visits<'a, F>(
    expression: &'a ConditionalExpr,
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_conditional_visits(&expr.left, options, context, visitor);
            collect_conditional_visits(&expr.right, options, context, visitor);
        }
        ConditionalExpr::Unary(expr) => {
            collect_conditional_visits(&expr.expr, options, context, visitor)
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_conditional_visits(&expr.expr, options, context, visitor);
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            collect_word_visits(word, options, context, visitor)
        }
        ConditionalExpr::Pattern(pattern) => {
            collect_pattern_visits(pattern, options, context, visitor)
        }
        ConditionalExpr::VarRef(reference) => {
            let mut subscript_words = Vec::new();
            collect_var_ref_subscript_words(reference, &mut subscript_words);
            for word in subscript_words {
                collect_word_visits(word, options, context, visitor);
            }
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

fn collect_heredoc_body_part_visits<'a, F>(
    parts: &'a [HeredocBodyPartNode],
    options: CommandWalkOptions,
    context: CommandTraversalContext,
    visitor: &mut F,
) where
    F: FnMut(CommandVisit<'a>, CommandTraversalContext),
{
    if !options.descend_nested_word_commands {
        return;
    }

    for part in parts {
        match &part.kind {
            shuck_ast::HeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    let mut arithmetic_words = Vec::new();
                    collect_optional_arithmetic_words(Some(expression_ast), &mut arithmetic_words);
                    for word in arithmetic_words {
                        collect_word_visits(word, options, context, visitor);
                    }
                }
            }
            shuck_ast::HeredocBodyPart::CommandSubstitution { body, .. } => {
                collect_command_visits(body, options, context, visitor);
            }
            shuck_ast::HeredocBodyPart::Literal(_)
            | shuck_ast::HeredocBodyPart::Variable(_)
            | shuck_ast::HeredocBodyPart::Parameter(_) => {}
        }
    }
}

#[cfg(test)]
mod traversal_tests {
    use shuck_ast::{
        BourneParameterExpansion, Command, ParameterExpansionSyntax, StmtSeq, VarRef, Word,
        WordPart,
    };
    use shuck_parser::parser::Parser;

    use super::{
        CommandWalkOptions, ConditionKind, iter_commands,
        visit_var_ref_subscript_words_with_source, walk_commands,
    };

    fn parse_commands(source: &str) -> StmtSeq {
        let output = Parser::new(source).parse().unwrap();
        output.file.body
    }

    fn static_word_owned_text(word: &Word, source: &str) -> Option<String> {
        word.try_static_text(source).map(|text| text.into_owned())
    }

    fn parameter_access_reference(word: &Word) -> &VarRef {
        let [part] = word.parts.as_slice() else {
            panic!("expected single parameter part");
        };
        let WordPart::Parameter(parameter) = &part.kind else {
            panic!("expected parameter part, got {:?}", part.kind);
        };
        let ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) =
            &parameter.syntax
        else {
            panic!("expected access expansion, got {:?}", parameter.syntax);
        };
        reference
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

            static_word_owned_text(&command.name, source)
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

            static_word_owned_text(&command.name, source)
        })
        .collect::<Vec<_>>();

        assert_eq!(structural, vec!["echo"]);
        assert_eq!(nested, vec!["echo", "printf"]);
    }

    #[test]
    fn walk_commands_tracks_nested_word_and_condition_context() {
        let source = "\
if foo \"$(bar)\"; then
  :
elif while baz \"$(qux)\"; do :; done; then
  :
fi
";
        let commands = parse_commands(source);
        let mut visits = Vec::new();

        walk_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
            &mut |visit, context| {
                let Command::Simple(command) = visit.command else {
                    return;
                };
                let Some(name) = static_word_owned_text(&command.name, source) else {
                    return;
                };
                if name == ":" {
                    return;
                }
                visits.push((
                    name,
                    context.nested_word_command,
                    context.condition_kind,
                    context.in_if_condition,
                    context.in_elif_condition,
                ));
            },
        );

        assert_eq!(
            visits,
            vec![
                (
                    "foo".to_owned(),
                    false,
                    Some(ConditionKind::If),
                    true,
                    false
                ),
                ("bar".to_owned(), true, Some(ConditionKind::If), true, false),
                (
                    "baz".to_owned(),
                    false,
                    Some(ConditionKind::While),
                    true,
                    true
                ),
                (
                    "qux".to_owned(),
                    true,
                    Some(ConditionKind::While),
                    true,
                    true
                ),
            ]
        );
    }

    #[test]
    fn walk_commands_prefers_nested_if_and_elif_condition_kinds_inside_loops() {
        let source = "\
while if foo; then bar; elif baz; then qux; fi; do
  :
done
";
        let commands = parse_commands(source);
        let mut visits = Vec::new();

        walk_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
            &mut |visit, context| {
                let Command::Simple(command) = visit.command else {
                    return;
                };
                let Some(name) = static_word_owned_text(&command.name, source) else {
                    return;
                };
                if name == ":" {
                    return;
                }
                visits.push((
                    name,
                    context.condition_kind,
                    context.in_if_condition,
                    context.in_elif_condition,
                ));
            },
        );

        assert_eq!(
            visits,
            vec![
                ("foo".to_owned(), Some(ConditionKind::If), true, false),
                ("bar".to_owned(), Some(ConditionKind::While), false, false),
                ("baz".to_owned(), Some(ConditionKind::Elif), true, true),
                ("qux".to_owned(), Some(ConditionKind::While), false, false),
            ]
        );
    }

    #[test]
    fn walk_commands_descends_into_parameter_expansion_operands() {
        let source = "\
printf '%s\\n' ${value:-$(expr $(nproc) + 1)}
";
        let commands = parse_commands(source);
        let mut visits = Vec::new();

        walk_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
            &mut |visit, context| {
                let Command::Simple(command) = visit.command else {
                    return;
                };
                let Some(name) = static_word_owned_text(&command.name, source) else {
                    return;
                };
                visits.push((name, context.nested_word_command));
            },
        );

        assert_eq!(
            visits,
            vec![
                ("printf".to_owned(), false),
                ("expr".to_owned(), true),
                ("nproc".to_owned(), true),
            ]
        );
    }

    #[test]
    fn walk_commands_descends_into_parameter_replacement_words() {
        let source = "\
printf '%s\\n' \"${value/old/$(expr $(nproc) + 1)}\"\n\
";
        let commands = parse_commands(source);
        let mut visits = Vec::new();

        walk_commands(
            &commands,
            CommandWalkOptions {
                descend_nested_word_commands: true,
            },
            &mut |visit, context| {
                let Command::Simple(command) = visit.command else {
                    return;
                };
                let Some(name) = static_word_owned_text(&command.name, source) else {
                    return;
                };
                visits.push((name, context.nested_word_command));
            },
        );

        assert_eq!(
            visits,
            vec![
                ("printf".to_owned(), false),
                ("expr".to_owned(), true),
                ("nproc".to_owned(), true),
            ]
        );
    }

    #[test]
    fn visit_var_ref_subscript_words_uses_parser_backed_subscript_words() {
        let source = "echo ${map[$key]} ${map[@]}\n";
        let commands = parse_commands(source);
        let Command::Simple(command) = &commands.stmts[0].command else {
            panic!("expected simple command");
        };

        let ordinary = parameter_access_reference(&command.args[0]);
        let selector = parameter_access_reference(&command.args[1]);

        let mut ordinary_words = Vec::new();
        visit_var_ref_subscript_words_with_source(ordinary, source, &mut |word| {
            ordinary_words.push(word.render_syntax(source));
        });

        let mut selector_words = Vec::new();
        visit_var_ref_subscript_words_with_source(selector, source, &mut |word| {
            selector_words.push(word.render_syntax(source));
        });

        assert_eq!(ordinary_words, vec!["$key"]);
        assert!(selector_words.is_empty());
    }
}
