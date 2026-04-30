// Shared command/word traversal helpers. Normal fact construction consumes semantic
// command visits rather than maintaining a separate recursive command walker.

#[derive(Debug, Clone, Copy)]
pub(super) struct CommandVisit<'a> {
    pub(super) stmt: &'a Stmt,
    pub(super) command: &'a Command,
    pub(super) redirects: &'a [Redirect],
}

impl<'a> CommandVisit<'a> {
    pub(super) fn new(stmt: &'a Stmt) -> Self {
        Self {
            stmt,
            command: &stmt.command,
            redirects: &stmt.redirects,
        }
    }
}

/// Controls traversal when walking nested statement-sequence bodies.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BodyTraversal {
    /// Visit nested bodies below the current body.
    Descend,
    /// Visit the current body but do not visit its nested bodies.
    SkipChildren,
    /// Stop body traversal immediately.
    Break,
}

/// Local topology view for one statement-sequence body.
///
/// Use this for direct statement order, sibling windows, and nested body traversal when semantic
/// command ids are not needed. Use `CommandTopology::body` instead when command identity or
/// syntax-backed parent/child relationships are part of the fact.
pub(super) struct BodyTopology<'a> {
    body: Option<&'a StmtSeq>,
    statements: &'a [Stmt],
}

impl<'a> BodyTopology<'a> {
    /// Creates a topology view over `body`.
    pub(super) fn new(body: &'a StmtSeq) -> Self {
        Self {
            body: Some(body),
            statements: body.as_slice(),
        }
    }

    /// Creates a topology view over an already sliced body segment.
    pub(super) fn from_statements(statements: &'a [Stmt]) -> Self {
        Self {
            body: None,
            statements,
        }
    }

    /// Returns direct statements in this body.
    pub(super) fn statements(&self) -> &'a [Stmt] {
        self.statements
    }

    /// Iterates adjacent direct statement pairs in source order.
    pub(super) fn sibling_pairs(&self) -> impl Iterator<Item = (&'a Stmt, &'a Stmt)> + '_ {
        self.indexed_sibling_pairs()
            .map(|(_, previous, current)| (previous, current))
    }

    /// Iterates adjacent direct statement pairs with the first statement's index.
    pub(super) fn indexed_sibling_pairs(
        &self,
    ) -> impl Iterator<Item = (usize, &'a Stmt, &'a Stmt)> + '_ {
        self.statements()
            .windows(2)
            .enumerate()
            .map(|(index, window)| (index, &window[0], &window[1]))
    }

    /// Returns the direct statement before `index`, if one exists.
    #[allow(dead_code)]
    pub(super) fn previous_sibling(&self, index: usize) -> Option<&'a Stmt> {
        index
            .checked_sub(1)
            .and_then(|previous| self.statements().get(previous))
    }

    /// Returns the direct statement after `index`, if one exists.
    #[allow(dead_code)]
    pub(super) fn next_sibling(&self, index: usize) -> Option<&'a Stmt> {
        self.statements().get(index + 1)
    }

    /// Visits this body and nested statement-sequence bodies in source order.
    ///
    /// The visitor controls whether nested bodies below each visited body should be traversed.
    /// This keeps skip behavior explicit at the call site instead of baking policy into another
    /// recursive helper.
    pub(super) fn for_each_body(
        &self,
        mut visitor: impl FnMut(&'a StmtSeq) -> BodyTraversal,
    ) {
        let Some(body) = self.body else {
            debug_assert!(
                false,
                "BodyTopology::for_each_body requires a complete StmtSeq"
            );
            return;
        };
        let mut stack = vec![body];
        while let Some(body) = stack.pop() {
            match visitor(body) {
                BodyTraversal::Descend => {}
                BodyTraversal::SkipChildren => continue,
                BodyTraversal::Break => break,
            }

            let mut nested_bodies = Vec::new();
            for stmt in body.iter() {
                Self::collect_nested_bodies_in_stmt(stmt, &mut nested_bodies);
            }
            for nested_body in nested_bodies.into_iter().rev() {
                stack.push(nested_body);
            }
        }
    }

    fn collect_nested_bodies_in_stmt(stmt: &'a Stmt, bodies: &mut Vec<&'a StmtSeq>) {
        match &stmt.command {
            Command::Binary(command) => {
                Self::collect_nested_bodies_in_stmt(&command.left, bodies);
                Self::collect_nested_bodies_in_stmt(&command.right, bodies);
            }
            Command::Compound(command) => match command {
                CompoundCommand::If(command) => {
                    bodies.push(&command.then_branch);
                    for (_, branch) in &command.elif_branches {
                        bodies.push(branch);
                    }
                    if let Some(branch) = &command.else_branch {
                        bodies.push(branch);
                    }
                }
                CompoundCommand::While(command) => bodies.push(&command.body),
                CompoundCommand::Until(command) => bodies.push(&command.body),
                CompoundCommand::For(command) => bodies.push(&command.body),
                CompoundCommand::Select(command) => bodies.push(&command.body),
                CompoundCommand::BraceGroup(body) | CompoundCommand::Subshell(body) => {
                    bodies.push(body);
                }
                CompoundCommand::Time(command) => {
                    if let Some(inner) = &command.command {
                        Self::collect_nested_bodies_in_stmt(inner, bodies);
                    }
                }
                CompoundCommand::Always(command) => {
                    bodies.push(&command.body);
                    bodies.push(&command.always_body);
                }
                CompoundCommand::Case(_)
                | CompoundCommand::Conditional(_)
                | CompoundCommand::Repeat(_)
                | CompoundCommand::Foreach(_)
                | CompoundCommand::ArithmeticFor(_)
                | CompoundCommand::Arithmetic(_)
                | CompoundCommand::Coproc(_) => {}
            },
            Command::Function(function) => {
                Self::collect_nested_bodies_in_stmt(&function.body, bodies);
            }
            Command::AnonymousFunction(function) => {
                Self::collect_nested_bodies_in_stmt(&function.body, bodies);
            }
            Command::Simple(_) | Command::Builtin(_) | Command::Decl(_) => {}
        }
    }
}

/// Kind of adjacent binary command chain to flatten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BinaryCommandChainKind {
    /// A chain formed from `&&` and `||` operators.
    LogicalList,
    /// A chain formed from `|` and `|&` operators.
    Pipeline,
}

impl BinaryCommandChainKind {
    fn contains(self, op: BinaryOp) -> bool {
        match self {
            Self::LogicalList => matches!(op, BinaryOp::And | BinaryOp::Or),
            Self::Pipeline => matches!(op, BinaryOp::Pipe | BinaryOp::PipeAll),
        }
    }
}

/// Source-order view of one adjacent binary command chain.
///
/// The helper flattens only compatible neighboring binary nodes. It does not classify segments,
/// inspect words, or recurse into unrelated command forms; callers keep that payload-specific
/// policy local to the owning fact.
#[derive(Debug, Clone, Copy)]
pub(super) struct BinaryCommandChain<'a> {
    root: &'a BinaryCommand,
    kind: BinaryCommandChainKind,
}

impl<'a> BinaryCommandChain<'a> {
    /// Returns a logical-list chain when `root` is joined by `&&` or `||`.
    pub(super) fn logical_list(root: &'a BinaryCommand) -> Option<Self> {
        Self::new(root, BinaryCommandChainKind::LogicalList)
    }

    /// Returns a pipeline chain when `root` is joined by `|` or `|&`.
    pub(super) fn pipeline(root: &'a BinaryCommand) -> Option<Self> {
        Self::new(root, BinaryCommandChainKind::Pipeline)
    }

    fn new(root: &'a BinaryCommand, kind: BinaryCommandChainKind) -> Option<Self> {
        kind.contains(root.op).then_some(Self { root, kind })
    }

    /// Visits leaf segments and chain operator nodes in source order.
    pub(super) fn visit_parts(
        &self,
        mut visit_segment: impl FnMut(&'a Stmt),
        mut visit_operator: impl FnMut(&'a BinaryCommand),
    ) {
        let mut stack = vec![BinaryChainStackItem::Command(self.root)];
        while let Some(item) = stack.pop() {
            match item {
                BinaryChainStackItem::Command(command) => {
                    match &command.right.command {
                        Command::Binary(right) if self.kind.contains(right.op) => {
                            stack.push(BinaryChainStackItem::Command(right));
                        }
                        _ => stack.push(BinaryChainStackItem::Segment(&command.right)),
                    }
                    stack.push(BinaryChainStackItem::Operator(command));
                    match &command.left.command {
                        Command::Binary(left) if self.kind.contains(left.op) => {
                            stack.push(BinaryChainStackItem::Command(left));
                        }
                        _ => stack.push(BinaryChainStackItem::Segment(&command.left)),
                    }
                }
                BinaryChainStackItem::Operator(command) => visit_operator(command),
                BinaryChainStackItem::Segment(stmt) => visit_segment(stmt),
            }
        }
    }

    /// Visits only leaf command segments in source order.
    pub(super) fn visit_segments(&self, visitor: impl FnMut(&'a Stmt)) {
        self.visit_parts(visitor, |_| {});
    }

    /// Visits only binary operator nodes in source order.
    pub(super) fn visit_nodes(&self, visitor: impl FnMut(&'a BinaryCommand)) {
        self.visit_parts(|_| {}, visitor);
    }
}

enum BinaryChainStackItem<'a> {
    Command(&'a BinaryCommand),
    Operator(&'a BinaryCommand),
    Segment(&'a Stmt),
}

fn visit_command_substitution_candidate_words<'a>(
    body: &'a StmtSeq,
    semantic: &LinterSemanticArtifacts<'a>,
    source: &str,
    visitor: &mut impl FnMut(&'a Word),
) {
    semantic
        .command_topology()
        .body(body)
        .for_each_command_visit(true, |_, visit| {
            visit_command_substitution_loop_header_words(visit.command, visitor);
            visit_command_argument_words_for_substitutions(visit.command, source, visitor);
            CommandTopologyTraversal::Descend
        });
}

fn visit_command_substitution_loop_header_words<'a>(
    command: &'a Command,
    visitor: &mut impl FnMut(&'a Word),
) {
    match command {
        Command::Compound(CompoundCommand::For(command)) => {
            if let Some(words) = &command.words {
                for word in words {
                    visitor(word);
                }
            }
        }
        Command::Compound(CompoundCommand::Select(command)) => {
            for word in &command.words {
                visitor(word);
            }
        }
        _ => {}
    }
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

fn visit_arithmetic_lvalue_words<'a>(
    target: &'a ArithmeticLvalue,
    visitor: &mut impl FnMut(&'a Word),
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => visit_arithmetic_words_in_expr(index, visitor),
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

#[cfg(test)]
mod traversal_tests {
    use shuck_ast::{
        BourneParameterExpansion, Command, ParameterExpansionSyntax, StmtSeq, VarRef, Word,
        WordPart,
    };
    use shuck_parser::parser::Parser;

    use super::visit_var_ref_subscript_words_with_source;

    fn parse_commands(source: &str) -> StmtSeq {
        let output = Parser::new(source).parse().unwrap();
        output.file.body
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
