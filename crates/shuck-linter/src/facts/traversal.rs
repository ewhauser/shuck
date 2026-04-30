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

enum BinaryChainStackItem<'a> {
    Command(&'a BinaryCommand),
    Operator(&'a BinaryCommand),
    Segment(&'a Stmt),
}

/// Visits the leaf segments and operators of a left/right-associative binary command chain.
///
/// List and pipeline facts both need the same topology question: flatten adjacent binary commands
/// with compatible operators while preserving source order. Keeping the mechanics here prevents
/// each fact family from maintaining its own recursive AST descent.
fn visit_binary_chain_parts<'a>(
    command: &'a BinaryCommand,
    mut is_chain_operator: impl FnMut(BinaryOp) -> bool,
    mut visit_segment: impl FnMut(&'a Stmt),
    mut visit_operator: impl FnMut(&'a BinaryCommand),
) {
    let mut stack = vec![BinaryChainStackItem::Command(command)];
    while let Some(item) = stack.pop() {
        match item {
            BinaryChainStackItem::Command(command) => {
                match &command.right.command {
                    Command::Binary(right) if is_chain_operator(right.op) => {
                        stack.push(BinaryChainStackItem::Command(right));
                    }
                    _ => stack.push(BinaryChainStackItem::Segment(&command.right)),
                }
                stack.push(BinaryChainStackItem::Operator(command));
                match &command.left.command {
                    Command::Binary(left) if is_chain_operator(left.op) => {
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

fn visit_command_substitution_candidate_words<'a>(
    body: &'a StmtSeq,
    semantic: &LinterSemanticArtifacts<'a>,
    source: &str,
    visitor: &mut impl FnMut(&'a Word),
) {
    semantic.for_each_command_visit_in_body(body, true, |visit| {
        visit_command_substitution_loop_header_words(visit.command, visitor);
        visit_command_argument_words_for_substitutions(visit.command, source, visitor);
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
