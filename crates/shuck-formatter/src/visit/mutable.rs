use super::*;

pub(crate) fn walk_file_mut<V: AstVisitorMut + ?Sized>(visitor: &mut V, file: &mut File) -> usize {
    visitor.visit_stmt_seq(&mut file.body)
}

pub(crate) fn walk_stmt_seq_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    sequence: &mut StmtSeq,
) -> usize {
    sequence
        .iter_mut()
        .map(|stmt| visitor.visit_stmt(stmt))
        .sum()
}

pub(crate) fn walk_stmt_mut<V: AstVisitorMut + ?Sized>(visitor: &mut V, stmt: &mut Stmt) -> usize {
    let mut changes = visitor.visit_command(&mut stmt.command);
    for redirect in &mut stmt.redirects {
        changes += visitor.visit_redirect(redirect);
    }
    changes
}

pub(crate) fn walk_command_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    command: &mut Command,
) -> usize {
    match command {
        Command::Simple(command) => {
            let mut changes = 0;
            for assignment in &mut command.assignments {
                changes += visitor.visit_assignment(assignment);
            }
            changes += visitor.visit_word(&mut command.name);
            for word in &mut command.args {
                changes += visitor.visit_word(word);
            }
            changes
        }
        Command::Builtin(command) => {
            let (assignments, primary, extra_args) = builtin_like_parts_mut(command);
            let mut changes = 0;
            for assignment in assignments {
                changes += visitor.visit_assignment(assignment);
            }
            if let Some(primary) = primary {
                changes += visitor.visit_word(primary);
            }
            for word in extra_args {
                changes += visitor.visit_word(word);
            }
            changes
        }
        Command::Decl(command) => walk_decl_clause_mut(visitor, command),
        Command::Binary(command) => {
            visitor.visit_stmt(&mut command.left) + visitor.visit_stmt(&mut command.right)
        }
        Command::Compound(command) => visitor.visit_compound_command(command),
        Command::Function(function) => visitor.visit_function(function),
        Command::AnonymousFunction(function) => visitor.visit_anonymous_function(function),
    }
}

fn walk_decl_clause_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    command: &mut DeclClause,
) -> usize {
    let mut changes = 0;
    for assignment in &mut command.assignments {
        changes += visitor.visit_assignment(assignment);
    }
    for operand in &mut command.operands {
        changes += match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => visitor.visit_word(word),
            DeclOperand::Name(reference) => visitor.visit_var_ref(reference),
            DeclOperand::Assignment(assignment) => visitor.visit_assignment(assignment),
        };
    }
    changes
}

pub(crate) fn walk_compound_command_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    command: &mut CompoundCommand,
) -> usize {
    match command {
        CompoundCommand::If(command) => {
            let mut changes = visitor.visit_stmt_seq(&mut command.condition)
                + visitor.visit_stmt_seq(&mut command.then_branch);
            for (condition, body) in &mut command.elif_branches {
                changes += visitor.visit_stmt_seq(condition);
                changes += visitor.visit_stmt_seq(body);
            }
            if let Some(body) = &mut command.else_branch {
                changes += visitor.visit_stmt_seq(body);
            }
            changes
        }
        CompoundCommand::For(command) => {
            let mut changes = 0;
            for target in &mut command.targets {
                changes += visitor.visit_word(&mut target.word);
            }
            if let Some(words) = &mut command.words {
                for word in words {
                    changes += visitor.visit_word(word);
                }
            }
            changes + visitor.visit_stmt_seq(&mut command.body)
        }
        CompoundCommand::Repeat(command) => {
            visitor.visit_word(&mut command.count) + visitor.visit_stmt_seq(&mut command.body)
        }
        CompoundCommand::Foreach(command) => {
            let mut changes = 0;
            for word in &mut command.words {
                changes += visitor.visit_word(word);
            }
            changes + visitor.visit_stmt_seq(&mut command.body)
        }
        CompoundCommand::ArithmeticFor(command) => {
            let mut changes = 0;
            if let Some(expression) = &mut command.init_ast {
                changes += visitor.visit_arithmetic_expr(expression);
            }
            if let Some(expression) = &mut command.condition_ast {
                changes += visitor.visit_arithmetic_expr(expression);
            }
            if let Some(expression) = &mut command.step_ast {
                changes += visitor.visit_arithmetic_expr(expression);
            }
            changes + visitor.visit_stmt_seq(&mut command.body)
        }
        CompoundCommand::While(command) => {
            visitor.visit_stmt_seq(&mut command.condition)
                + visitor.visit_stmt_seq(&mut command.body)
        }
        CompoundCommand::Until(command) => {
            visitor.visit_stmt_seq(&mut command.condition)
                + visitor.visit_stmt_seq(&mut command.body)
        }
        CompoundCommand::Case(command) => {
            let mut changes = visitor.visit_word(&mut command.word);
            for item in &mut command.cases {
                for pattern in &mut item.patterns {
                    changes += visitor.visit_pattern(pattern);
                }
                changes += visitor.visit_stmt_seq(&mut item.body);
            }
            changes
        }
        CompoundCommand::Select(command) => {
            let mut changes = 0;
            for word in &mut command.words {
                changes += visitor.visit_word(word);
            }
            changes + visitor.visit_stmt_seq(&mut command.body)
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            visitor.visit_stmt_seq(commands)
        }
        CompoundCommand::Arithmetic(command) => command
            .expr_ast
            .as_mut()
            .map_or(0, |expression| visitor.visit_arithmetic_expr(expression)),
        CompoundCommand::Time(command) => command
            .command
            .as_mut()
            .map_or(0, |command| visitor.visit_stmt(command)),
        CompoundCommand::Conditional(command) => {
            visitor.visit_conditional_expr(&mut command.expression)
        }
        CompoundCommand::Coproc(command) => visitor.visit_stmt(&mut command.body),
        CompoundCommand::Always(command) => {
            visitor.visit_stmt_seq(&mut command.body)
                + visitor.visit_stmt_seq(&mut command.always_body)
        }
    }
}

pub(crate) fn walk_function_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    function: &mut FunctionDef,
) -> usize {
    let mut changes = 0;
    for entry in &mut function.header.entries {
        changes += visitor.visit_word(&mut entry.word);
    }
    changes + visitor.visit_stmt(&mut function.body)
}

pub(crate) fn walk_anonymous_function_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    function: &mut AnonymousFunctionCommand,
) -> usize {
    let mut changes = visitor.visit_stmt(&mut function.body);
    for argument in &mut function.args {
        changes += visitor.visit_word(argument);
    }
    changes
}

pub(crate) fn walk_redirect_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    redirect: &mut Redirect,
) -> usize {
    match &mut redirect.target {
        RedirectTarget::Word(word) => visitor.visit_word(word),
        RedirectTarget::Heredoc(heredoc) => visitor.visit_heredoc(heredoc),
    }
}

pub(crate) fn walk_heredoc_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    heredoc: &mut Heredoc,
) -> usize {
    visitor.visit_word(&mut heredoc.delimiter.raw) + visitor.visit_heredoc_body(&mut heredoc.body)
}

pub(crate) fn walk_assignment_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    assignment: &mut Assignment,
) -> usize {
    let mut changes = visitor.visit_var_ref(&mut assignment.target);
    changes += match &mut assignment.value {
        AssignmentValue::Scalar(word) => visitor.visit_word(word),
        AssignmentValue::Compound(array) => {
            let mut inner = 0;
            for element in &mut array.elements {
                match element {
                    shuck_ast::ArrayElem::Sequential(_) => {}
                    shuck_ast::ArrayElem::Keyed { key, .. }
                    | shuck_ast::ArrayElem::KeyedAppend { key, .. } => {
                        inner += visitor.visit_subscript(key);
                    }
                }
                inner += visitor.visit_word(array_elem_value_word_mut(element));
            }
            inner
        }
    };
    changes
}

pub(crate) fn walk_var_ref_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    reference: &mut VarRef,
) -> usize {
    reference
        .subscript
        .as_mut()
        .map_or(0, |subscript| visitor.visit_subscript(subscript))
}

pub(crate) fn walk_subscript_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    subscript: &mut Subscript,
) -> usize {
    let mut changes = visitor.visit_source_text(&mut subscript.text);
    if let Some(raw) = &mut subscript.raw {
        changes += visitor.visit_source_text(raw);
    }
    if let Some(word) = &mut subscript.word_ast {
        changes += visitor.visit_word(word);
    }
    if let Some(expression) = &mut subscript.arithmetic_ast {
        changes += visitor.visit_arithmetic_expr(expression);
    }
    changes
}

pub(crate) fn walk_conditional_expr_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    expression: &mut ConditionalExpr,
) -> usize {
    match expression {
        ConditionalExpr::Binary(expression) => {
            visitor.visit_conditional_expr(&mut expression.left)
                + visitor.visit_conditional_expr(&mut expression.right)
        }
        ConditionalExpr::Unary(expression) => visitor.visit_conditional_expr(&mut expression.expr),
        ConditionalExpr::Parenthesized(expression) => {
            visitor.visit_conditional_expr(&mut expression.expr)
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => visitor.visit_word(word),
        ConditionalExpr::Pattern(pattern) => visitor.visit_pattern(pattern),
        ConditionalExpr::VarRef(reference) => visitor.visit_var_ref(reference),
    }
}

pub(crate) fn walk_pattern_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    pattern: &mut Pattern,
) -> usize {
    pattern
        .parts
        .iter_mut()
        .map(|part| visitor.visit_pattern_part(part))
        .sum()
}

pub(crate) fn walk_pattern_part_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    part: &mut PatternPartNode,
) -> usize {
    match &mut part.kind {
        PatternPart::CharClass(text) => visitor.visit_source_text(text),
        PatternPart::Group { patterns, .. } => patterns
            .iter_mut()
            .map(|pattern| visitor.visit_pattern(pattern))
            .sum(),
        PatternPart::Word(word) => visitor.visit_word(word),
        PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => 0,
    }
}

pub(crate) fn walk_word_mut<V: AstVisitorMut + ?Sized>(visitor: &mut V, word: &mut Word) -> usize {
    word.parts
        .iter_mut()
        .map(|part| visitor.visit_word_part(part))
        .sum()
}

pub(crate) fn walk_word_part_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    part: &mut WordPartNode,
) -> usize {
    match &mut part.kind {
        WordPart::Literal(_) | WordPart::Variable(_) | WordPart::PrefixMatch { .. } => 0,
        WordPart::ZshQualifiedGlob(glob) => walk_zsh_qualified_glob_mut(visitor, glob),
        WordPart::SingleQuoted { value, .. } => visitor.visit_source_text(value),
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter_mut()
            .map(|part| visitor.visit_word_part(part))
            .sum(),
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            visitor.visit_stmt_seq(body)
        }
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            expression_word_ast,
            ..
        } => {
            let mut changes = visitor.visit_source_text(expression);
            if let Some(expression) = expression_ast {
                changes += visitor.visit_arithmetic_expr(expression);
            } else {
                changes += visitor.visit_word(expression_word_ast);
            }
            changes
        }
        WordPart::Parameter(parameter) => visitor.visit_parameter_expansion(parameter),
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            operand_word_ast,
            ..
        } => {
            let mut changes =
                visitor.visit_var_ref(reference) + visitor.visit_parameter_op(operator);
            if let Some(operand) = operand {
                changes += visitor.visit_source_text(operand);
            }
            if let Some(operand) = operand_word_ast {
                changes += visitor.visit_word(operand);
            }
            changes
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => visitor.visit_var_ref(reference),
        WordPart::Substring {
            reference,
            offset,
            offset_ast,
            offset_word_ast,
            length,
            length_ast,
            length_word_ast,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset,
            offset_ast,
            offset_word_ast,
            length,
            length_ast,
            length_word_ast,
            ..
        } => {
            let mut changes = visitor.visit_var_ref(reference) + visitor.visit_source_text(offset);
            if let Some(expression) = offset_ast {
                changes += visitor.visit_arithmetic_expr(expression);
            } else {
                changes += visitor.visit_word(offset_word_ast);
            }
            if let Some(length) = length {
                changes += visitor.visit_source_text(length);
            }
            if let Some(expression) = length_ast {
                changes += visitor.visit_arithmetic_expr(expression);
            } else if let Some(word) = length_word_ast {
                changes += visitor.visit_word(word);
            }
            changes
        }
        WordPart::IndirectExpansion {
            reference,
            operator,
            operand,
            operand_word_ast,
            ..
        } => {
            let mut changes = visitor.visit_var_ref(reference);
            if let Some(operator) = operator {
                changes += visitor.visit_parameter_op(operator);
            }
            if let Some(operand) = operand {
                changes += visitor.visit_source_text(operand);
            }
            if let Some(operand) = operand_word_ast {
                changes += visitor.visit_word(operand);
            }
            changes
        }
    }
}

pub(crate) fn walk_heredoc_body_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    body: &mut HeredocBody,
) -> usize {
    body.parts
        .iter_mut()
        .map(|part| visitor.visit_heredoc_body_part(part))
        .sum()
}

pub(crate) fn walk_heredoc_body_part_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    part: &mut HeredocBodyPartNode,
) -> usize {
    match &mut part.kind {
        HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => 0,
        HeredocBodyPart::CommandSubstitution { body, .. } => visitor.visit_stmt_seq(body),
        HeredocBodyPart::ArithmeticExpansion {
            expression,
            expression_ast,
            expression_word_ast,
            ..
        } => {
            let mut changes = visitor.visit_source_text(expression);
            if let Some(expression) = expression_ast {
                changes += visitor.visit_arithmetic_expr(expression);
            } else {
                changes += visitor.visit_word(expression_word_ast);
            }
            changes
        }
        HeredocBodyPart::Parameter(parameter) => visitor.visit_parameter_expansion(parameter),
    }
}

pub(crate) fn walk_arithmetic_expr_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    expression: &mut ArithmeticExprNode,
) -> usize {
    match &mut expression.kind {
        ArithmeticExpr::Number(text) => visitor.visit_source_text(text),
        ArithmeticExpr::Variable(_) => 0,
        ArithmeticExpr::Indexed { index, .. } => visitor.visit_arithmetic_expr(index),
        ArithmeticExpr::ShellWord(word) => visitor.visit_word(word),
        ArithmeticExpr::Parenthesized { expression } => visitor.visit_arithmetic_expr(expression),
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            visitor.visit_arithmetic_expr(expr)
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            visitor.visit_arithmetic_expr(left) + visitor.visit_arithmetic_expr(right)
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visitor.visit_arithmetic_expr(condition)
                + visitor.visit_arithmetic_expr(then_expr)
                + visitor.visit_arithmetic_expr(else_expr)
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            visitor.visit_arithmetic_lvalue(target) + visitor.visit_arithmetic_expr(value)
        }
    }
}

pub(crate) fn walk_arithmetic_lvalue_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    target: &mut ArithmeticLvalue,
) -> usize {
    match target {
        ArithmeticLvalue::Variable(_) => 0,
        ArithmeticLvalue::Indexed { index, .. } => visitor.visit_arithmetic_expr(index),
    }
}

pub(crate) fn walk_parameter_expansion_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    parameter: &mut ParameterExpansion,
) -> usize {
    match &mut parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                visitor.visit_var_ref(reference)
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                let mut changes = visitor.visit_var_ref(reference);
                if let Some(operator) = operator {
                    changes += visitor.visit_parameter_op(operator);
                }
                if let Some(operand) = operand {
                    changes += visitor.visit_source_text(operand);
                }
                if let Some(operand) = operand_word_ast {
                    changes += visitor.visit_word(operand);
                }
                changes
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                let mut changes =
                    visitor.visit_var_ref(reference) + visitor.visit_parameter_op(operator);
                if let Some(operand) = operand {
                    changes += visitor.visit_source_text(operand);
                }
                if let Some(operand) = operand_word_ast {
                    changes += visitor.visit_word(operand);
                }
                changes
            }
            BourneParameterExpansion::PrefixMatch { .. } => 0,
            BourneParameterExpansion::Slice {
                reference,
                offset,
                offset_ast,
                offset_word_ast,
                length,
                length_ast,
                length_word_ast,
                ..
            } => {
                let mut changes =
                    visitor.visit_var_ref(reference) + visitor.visit_source_text(offset);
                if let Some(expression) = offset_ast {
                    changes += visitor.visit_arithmetic_expr(expression);
                } else {
                    changes += visitor.visit_word(offset_word_ast);
                }
                if let Some(length) = length {
                    changes += visitor.visit_source_text(length);
                }
                if let Some(expression) = length_ast {
                    changes += visitor.visit_arithmetic_expr(expression);
                } else if let Some(word) = length_word_ast {
                    changes += visitor.visit_word(word);
                }
                changes
            }
        },
        ParameterExpansionSyntax::Zsh(syntax) => walk_zsh_parameter_expansion_mut(visitor, syntax),
    }
}

fn walk_zsh_parameter_expansion_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    syntax: &mut ZshParameterExpansion,
) -> usize {
    let mut changes = match &mut syntax.target {
        ZshExpansionTarget::Reference(reference) => visitor.visit_var_ref(reference),
        ZshExpansionTarget::Nested(parameter) => visitor.visit_parameter_expansion(parameter),
        ZshExpansionTarget::Word(word) => visitor.visit_word(word),
        ZshExpansionTarget::Empty => 0,
    };
    for modifier in &mut syntax.modifiers {
        if let Some(argument) = &mut modifier.argument {
            changes += visitor.visit_source_text(argument);
        }
        if let Some(word) = &mut modifier.argument_word_ast {
            changes += visitor.visit_word(word);
        }
    }
    if let Some(operation) = &mut syntax.operation {
        changes += walk_zsh_expansion_operation_mut(visitor, operation);
    }
    changes
}

fn walk_zsh_expansion_operation_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    operation: &mut ZshExpansionOperation,
) -> usize {
    match operation {
        ZshExpansionOperation::PatternOperation {
            operand,
            operand_word_ast,
            ..
        }
        | ZshExpansionOperation::Defaulting {
            operand,
            operand_word_ast,
            ..
        }
        | ZshExpansionOperation::TrimOperation {
            operand,
            operand_word_ast,
            ..
        } => visitor.visit_source_text(operand) + visitor.visit_word(operand_word_ast),
        ZshExpansionOperation::ReplacementOperation {
            pattern,
            pattern_word_ast,
            replacement,
            replacement_word_ast,
            ..
        } => {
            let mut changes =
                visitor.visit_source_text(pattern) + visitor.visit_word(pattern_word_ast);
            if let Some(replacement) = replacement {
                changes += visitor.visit_source_text(replacement);
            }
            if let Some(replacement) = replacement_word_ast {
                changes += visitor.visit_word(replacement);
            }
            changes
        }
        ZshExpansionOperation::Slice {
            offset,
            offset_word_ast,
            length,
            length_word_ast,
        } => {
            let mut changes =
                visitor.visit_source_text(offset) + visitor.visit_word(offset_word_ast);
            if let Some(length) = length {
                changes += visitor.visit_source_text(length);
            }
            if let Some(length) = length_word_ast {
                changes += visitor.visit_word(length);
            }
            changes
        }
        ZshExpansionOperation::Unknown { text, word_ast } => {
            visitor.visit_source_text(text) + visitor.visit_word(word_ast)
        }
    }
}

pub(crate) fn walk_parameter_op_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    operator: &mut ParameterOp,
) -> usize {
    match operator {
        ParameterOp::RemovePrefixShort { pattern }
        | ParameterOp::RemovePrefixLong { pattern }
        | ParameterOp::RemoveSuffixShort { pattern }
        | ParameterOp::RemoveSuffixLong { pattern } => visitor.visit_pattern(pattern),
        ParameterOp::ReplaceFirst {
            pattern,
            replacement,
            replacement_word_ast,
        }
        | ParameterOp::ReplaceAll {
            pattern,
            replacement,
            replacement_word_ast,
        } => {
            visitor.visit_pattern(pattern)
                + visitor.visit_source_text(replacement)
                + visitor.visit_word(replacement_word_ast)
        }
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => 0,
    }
}

fn walk_zsh_qualified_glob_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    glob: &mut shuck_ast::ZshQualifiedGlob,
) -> usize {
    let mut changes = 0;
    for segment in &mut glob.segments {
        changes += match segment {
            ZshGlobSegment::Pattern(pattern) => visitor.visit_pattern(pattern),
            ZshGlobSegment::InlineControl(_) => 0,
        };
    }
    if let Some(qualifiers) = &mut glob.qualifiers {
        changes += walk_zsh_glob_qualifier_group_mut(visitor, qualifiers);
    }
    changes
}

fn walk_zsh_glob_qualifier_group_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    group: &mut ZshGlobQualifierGroup,
) -> usize {
    group
        .fragments
        .iter_mut()
        .map(|fragment| match fragment {
            ZshGlobQualifier::LetterSequence { text, .. } => visitor.visit_source_text(text),
            ZshGlobQualifier::NumericArgument { start, end, .. } => {
                let mut changes = visitor.visit_source_text(start);
                if let Some(end) = end {
                    changes += visitor.visit_source_text(end);
                }
                changes
            }
            ZshGlobQualifier::Negation { .. } | ZshGlobQualifier::Flag { .. } => 0,
        })
        .sum()
}
