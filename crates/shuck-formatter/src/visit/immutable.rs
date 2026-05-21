use super::*;

pub(crate) fn walk_stmt_seq<V: AstVisitor + ?Sized>(visitor: &mut V, sequence: &StmtSeq) {
    for stmt in sequence.iter() {
        visitor.visit_stmt(stmt);
    }
}

pub(crate) fn walk_stmt<V: AstVisitor + ?Sized>(visitor: &mut V, stmt: &Stmt) {
    visitor.visit_command(&stmt.command);
    for redirect in &stmt.redirects {
        visitor.visit_redirect(redirect);
    }
}

pub(crate) fn walk_command<V: AstVisitor + ?Sized>(visitor: &mut V, command: &Command) {
    match command {
        Command::Simple(command) => {
            for assignment in &command.assignments {
                visitor.visit_assignment(assignment);
            }
            visitor.visit_word(&command.name);
            for word in &command.args {
                visitor.visit_word(word);
            }
        }
        Command::Builtin(command) => {
            let (_, _, assignments, primary, extra_args) = builtin_like_parts(command);
            for assignment in assignments {
                visitor.visit_assignment(assignment);
            }
            if let Some(primary) = primary {
                visitor.visit_word(primary);
            }
            for word in extra_args {
                visitor.visit_word(word);
            }
        }
        Command::Decl(command) => walk_decl_clause(visitor, command),
        Command::Binary(command) => {
            visitor.visit_stmt(&command.left);
            visitor.visit_stmt(&command.right);
        }
        Command::Compound(command) => visitor.visit_compound_command(command),
        Command::Function(function) => visitor.visit_function(function),
        Command::AnonymousFunction(function) => visitor.visit_anonymous_function(function),
    }
}

fn walk_decl_clause<V: AstVisitor + ?Sized>(visitor: &mut V, command: &DeclClause) {
    for assignment in &command.assignments {
        visitor.visit_assignment(assignment);
    }
    for operand in &command.operands {
        match operand {
            DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => visitor.visit_word(word),
            DeclOperand::Name(reference) => visitor.visit_var_ref(reference),
            DeclOperand::Assignment(assignment) => visitor.visit_assignment(assignment),
        }
    }
}

pub(crate) fn walk_compound_command<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    command: &CompoundCommand,
) {
    match command {
        CompoundCommand::If(command) => {
            visitor.visit_stmt_seq(&command.condition);
            visitor.visit_stmt_seq(&command.then_branch);
            for (condition, body) in &command.elif_branches {
                visitor.visit_stmt_seq(condition);
                visitor.visit_stmt_seq(body);
            }
            if let Some(body) = &command.else_branch {
                visitor.visit_stmt_seq(body);
            }
        }
        CompoundCommand::For(command) => {
            for target in &command.targets {
                visitor.visit_word(&target.word);
            }
            if let Some(words) = &command.words {
                for word in words {
                    visitor.visit_word(word);
                }
            }
            visitor.visit_stmt_seq(&command.body);
        }
        CompoundCommand::Repeat(command) => {
            visitor.visit_word(&command.count);
            visitor.visit_stmt_seq(&command.body);
        }
        CompoundCommand::Foreach(command) => {
            for word in &command.words {
                visitor.visit_word(word);
            }
            visitor.visit_stmt_seq(&command.body);
        }
        CompoundCommand::ArithmeticFor(command) => {
            if let Some(expression) = &command.init_ast {
                visitor.visit_arithmetic_expr(expression);
            }
            if let Some(expression) = &command.condition_ast {
                visitor.visit_arithmetic_expr(expression);
            }
            if let Some(expression) = &command.step_ast {
                visitor.visit_arithmetic_expr(expression);
            }
            visitor.visit_stmt_seq(&command.body);
        }
        CompoundCommand::While(command) => {
            visitor.visit_stmt_seq(&command.condition);
            visitor.visit_stmt_seq(&command.body);
        }
        CompoundCommand::Until(command) => {
            visitor.visit_stmt_seq(&command.condition);
            visitor.visit_stmt_seq(&command.body);
        }
        CompoundCommand::Case(command) => {
            visitor.visit_word(&command.word);
            for item in &command.cases {
                for pattern in &item.patterns {
                    visitor.visit_pattern(pattern);
                }
                visitor.visit_stmt_seq(&item.body);
            }
        }
        CompoundCommand::Select(command) => {
            for word in &command.words {
                visitor.visit_word(word);
            }
            visitor.visit_stmt_seq(&command.body);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            visitor.visit_stmt_seq(commands);
        }
        CompoundCommand::Arithmetic(command) => {
            if let Some(expression) = &command.expr_ast {
                visitor.visit_arithmetic_expr(expression);
            }
        }
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                visitor.visit_stmt(command);
            }
        }
        CompoundCommand::Conditional(command) => {
            visitor.visit_conditional_expr(&command.expression)
        }
        CompoundCommand::Coproc(command) => visitor.visit_stmt(&command.body),
        CompoundCommand::Always(command) => {
            visitor.visit_stmt_seq(&command.body);
            visitor.visit_stmt_seq(&command.always_body);
        }
    }
}

pub(crate) fn walk_function<V: AstVisitor + ?Sized>(visitor: &mut V, function: &FunctionDef) {
    for entry in &function.header.entries {
        visitor.visit_word(&entry.word);
    }
    visitor.visit_stmt(&function.body);
}

pub(crate) fn walk_anonymous_function<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    function: &AnonymousFunctionCommand,
) {
    visitor.visit_stmt(&function.body);
    for argument in &function.args {
        visitor.visit_word(argument);
    }
}

pub(crate) fn walk_redirect<V: AstVisitor + ?Sized>(visitor: &mut V, redirect: &Redirect) {
    match &redirect.target {
        RedirectTarget::Word(word) => visitor.visit_word(word),
        RedirectTarget::Heredoc(heredoc) => visitor.visit_heredoc(heredoc),
    }
}

pub(crate) fn walk_heredoc<V: AstVisitor + ?Sized>(visitor: &mut V, heredoc: &Heredoc) {
    visitor.visit_word(&heredoc.delimiter.raw);
    visitor.visit_heredoc_body(&heredoc.body);
}

pub(crate) fn walk_assignment<V: AstVisitor + ?Sized>(visitor: &mut V, assignment: &Assignment) {
    visitor.visit_var_ref(&assignment.target);
    match &assignment.value {
        AssignmentValue::Scalar(word) => visitor.visit_word(word),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                if let Some(key) = array_elem_parts(element).0 {
                    visitor.visit_subscript(key);
                }
                visitor.visit_word(array_elem_parts(element).1);
            }
        }
    }
}

pub(crate) fn walk_var_ref<V: AstVisitor + ?Sized>(visitor: &mut V, reference: &VarRef) {
    if let Some(subscript) = &reference.subscript {
        visitor.visit_subscript(subscript);
    }
}

pub(crate) fn walk_subscript<V: AstVisitor + ?Sized>(visitor: &mut V, subscript: &Subscript) {
    visitor.visit_source_text(&subscript.text);
    if let Some(raw) = &subscript.raw {
        visitor.visit_source_text(raw);
    }
    if let Some(word) = &subscript.word_ast {
        visitor.visit_word(word);
    }
    if let Some(expression) = &subscript.arithmetic_ast {
        visitor.visit_arithmetic_expr(expression);
    }
}

pub(crate) fn walk_conditional_expr<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    expression: &ConditionalExpr,
) {
    match expression {
        ConditionalExpr::Binary(expression) => {
            visitor.visit_conditional_expr(&expression.left);
            visitor.visit_conditional_expr(&expression.right);
        }
        ConditionalExpr::Unary(expression) => visitor.visit_conditional_expr(&expression.expr),
        ConditionalExpr::Parenthesized(expression) => {
            visitor.visit_conditional_expr(&expression.expr);
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => visitor.visit_word(word),
        ConditionalExpr::Pattern(pattern) => visitor.visit_pattern(pattern),
        ConditionalExpr::VarRef(reference) => visitor.visit_var_ref(reference),
    }
}

pub(crate) fn walk_pattern<V: AstVisitor + ?Sized>(visitor: &mut V, pattern: &Pattern) {
    for part in &pattern.parts {
        visitor.visit_pattern_part(part);
    }
}

pub(crate) fn walk_pattern_part<V: AstVisitor + ?Sized>(visitor: &mut V, part: &PatternPartNode) {
    match &part.kind {
        PatternPart::CharClass(text) => visitor.visit_source_text(text),
        PatternPart::Group { patterns, .. } => {
            for pattern in patterns {
                visitor.visit_pattern(pattern);
            }
        }
        PatternPart::Word(word) => visitor.visit_word(word),
        PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => {}
    }
}

pub(crate) fn walk_word<V: AstVisitor + ?Sized>(visitor: &mut V, word: &Word) {
    for part in &word.parts {
        visitor.visit_word_part(part);
    }
}

pub(crate) fn walk_word_part<V: AstVisitor + ?Sized>(visitor: &mut V, part: &WordPartNode) {
    match &part.kind {
        WordPart::Literal(_) | WordPart::Variable(_) | WordPart::PrefixMatch { .. } => {}
        WordPart::ZshQualifiedGlob(glob) => walk_zsh_qualified_glob(visitor, glob),
        WordPart::SingleQuoted { value, .. } => visitor.visit_source_text(value),
        WordPart::DoubleQuoted { parts, .. } => {
            for part in parts {
                visitor.visit_word_part(part);
            }
        }
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            visitor.visit_stmt_seq(body);
        }
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            expression_word_ast,
            ..
        } => {
            visitor.visit_source_text(expression);
            if let Some(expression) = expression_ast {
                visitor.visit_arithmetic_expr(expression);
            } else {
                visitor.visit_word(expression_word_ast);
            }
        }
        WordPart::Parameter(parameter) => visitor.visit_parameter_expansion(parameter),
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            operand_word_ast,
            ..
        } => {
            visitor.visit_var_ref(reference);
            visitor.visit_parameter_op(operator);
            if let Some(operand) = operand {
                visitor.visit_source_text(operand);
            }
            if let Some(operand) = operand_word_ast {
                visitor.visit_word(operand);
            }
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
            visitor.visit_var_ref(reference);
            visitor.visit_source_text(offset);
            if let Some(expression) = offset_ast {
                visitor.visit_arithmetic_expr(expression);
            } else {
                visitor.visit_word(offset_word_ast);
            }
            if let Some(length) = length {
                visitor.visit_source_text(length);
            }
            if let Some(expression) = length_ast {
                visitor.visit_arithmetic_expr(expression);
            } else if let Some(word) = length_word_ast {
                visitor.visit_word(word);
            }
        }
        WordPart::IndirectExpansion {
            reference,
            operator,
            operand,
            operand_word_ast,
            ..
        } => {
            visitor.visit_var_ref(reference);
            if let Some(operator) = operator {
                visitor.visit_parameter_op(operator);
            }
            if let Some(operand) = operand {
                visitor.visit_source_text(operand);
            }
            if let Some(operand) = operand_word_ast {
                visitor.visit_word(operand);
            }
        }
    }
}

pub(crate) fn walk_heredoc_body<V: AstVisitor + ?Sized>(visitor: &mut V, body: &HeredocBody) {
    for part in &body.parts {
        visitor.visit_heredoc_body_part(part);
    }
}

pub(crate) fn walk_heredoc_body_part<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    part: &HeredocBodyPartNode,
) {
    match &part.kind {
        HeredocBodyPart::Literal(_) | HeredocBodyPart::Variable(_) => {}
        HeredocBodyPart::CommandSubstitution { body, .. } => visitor.visit_stmt_seq(body),
        HeredocBodyPart::ArithmeticExpansion {
            expression,
            expression_ast,
            expression_word_ast,
            ..
        } => {
            visitor.visit_source_text(expression);
            if let Some(expression) = expression_ast {
                visitor.visit_arithmetic_expr(expression);
            } else {
                visitor.visit_word(expression_word_ast);
            }
        }
        HeredocBodyPart::Parameter(parameter) => visitor.visit_parameter_expansion(parameter),
    }
}

pub(crate) fn walk_arithmetic_expr<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    expression: &ArithmeticExprNode,
) {
    match &expression.kind {
        ArithmeticExpr::Number(text) => visitor.visit_source_text(text),
        ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => visitor.visit_arithmetic_expr(index),
        ArithmeticExpr::ShellWord(word) => visitor.visit_word(word),
        ArithmeticExpr::Parenthesized { expression } => visitor.visit_arithmetic_expr(expression),
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            visitor.visit_arithmetic_expr(expr);
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            visitor.visit_arithmetic_expr(left);
            visitor.visit_arithmetic_expr(right);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            visitor.visit_arithmetic_expr(condition);
            visitor.visit_arithmetic_expr(then_expr);
            visitor.visit_arithmetic_expr(else_expr);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            visitor.visit_arithmetic_lvalue(target);
            visitor.visit_arithmetic_expr(value);
        }
    }
}

pub(crate) fn walk_arithmetic_lvalue<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    target: &ArithmeticLvalue,
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => visitor.visit_arithmetic_expr(index),
    }
}

pub(crate) fn walk_parameter_expansion<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    parameter: &ParameterExpansion,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                visitor.visit_var_ref(reference);
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                visitor.visit_var_ref(reference);
                if let Some(operator) = operator {
                    visitor.visit_parameter_op(operator);
                }
                if let Some(operand) = operand {
                    visitor.visit_source_text(operand);
                }
                if let Some(operand) = operand_word_ast {
                    visitor.visit_word(operand);
                }
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                visitor.visit_var_ref(reference);
                visitor.visit_parameter_op(operator);
                if let Some(operand) = operand {
                    visitor.visit_source_text(operand);
                }
                if let Some(operand) = operand_word_ast {
                    visitor.visit_word(operand);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
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
                visitor.visit_var_ref(reference);
                visitor.visit_source_text(offset);
                if let Some(expression) = offset_ast {
                    visitor.visit_arithmetic_expr(expression);
                } else {
                    visitor.visit_word(offset_word_ast);
                }
                if let Some(length) = length {
                    visitor.visit_source_text(length);
                }
                if let Some(expression) = length_ast {
                    visitor.visit_arithmetic_expr(expression);
                } else if let Some(word) = length_word_ast {
                    visitor.visit_word(word);
                }
            }
        },
        ParameterExpansionSyntax::Zsh(syntax) => walk_zsh_parameter_expansion(visitor, syntax),
    }
}

fn walk_zsh_parameter_expansion<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    syntax: &ZshParameterExpansion,
) {
    match &syntax.target {
        ZshExpansionTarget::Reference(reference) => visitor.visit_var_ref(reference),
        ZshExpansionTarget::Nested(parameter) => visitor.visit_parameter_expansion(parameter),
        ZshExpansionTarget::Word(word) => visitor.visit_word(word),
        ZshExpansionTarget::Empty => {}
    }
    for modifier in &syntax.modifiers {
        if let Some(argument) = &modifier.argument {
            visitor.visit_source_text(argument);
        }
        if let Some(word) = &modifier.argument_word_ast {
            visitor.visit_word(word);
        }
    }
    if let Some(operation) = &syntax.operation {
        walk_zsh_expansion_operation(visitor, operation);
    }
}

fn walk_zsh_expansion_operation<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    operation: &ZshExpansionOperation,
) {
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
        } => {
            visitor.visit_source_text(operand);
            visitor.visit_word(operand_word_ast);
        }
        ZshExpansionOperation::ReplacementOperation {
            pattern,
            pattern_word_ast,
            replacement,
            replacement_word_ast,
            ..
        } => {
            visitor.visit_source_text(pattern);
            visitor.visit_word(pattern_word_ast);
            if let Some(replacement) = replacement {
                visitor.visit_source_text(replacement);
            }
            if let Some(replacement) = replacement_word_ast {
                visitor.visit_word(replacement);
            }
        }
        ZshExpansionOperation::Slice {
            offset,
            offset_word_ast,
            length,
            length_word_ast,
        } => {
            visitor.visit_source_text(offset);
            visitor.visit_word(offset_word_ast);
            if let Some(length) = length {
                visitor.visit_source_text(length);
            }
            if let Some(length) = length_word_ast {
                visitor.visit_word(length);
            }
        }
        ZshExpansionOperation::Unknown { text, word_ast } => {
            visitor.visit_source_text(text);
            visitor.visit_word(word_ast);
        }
    }
}

pub(crate) fn walk_parameter_op<V: AstVisitor + ?Sized>(visitor: &mut V, operator: &ParameterOp) {
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
            visitor.visit_pattern(pattern);
            visitor.visit_source_text(replacement);
            visitor.visit_word(replacement_word_ast);
        }
        ParameterOp::UseDefault
        | ParameterOp::AssignDefault
        | ParameterOp::UseReplacement
        | ParameterOp::Error
        | ParameterOp::UpperFirst
        | ParameterOp::UpperAll
        | ParameterOp::LowerFirst
        | ParameterOp::LowerAll => {}
    }
}

fn walk_zsh_qualified_glob<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    glob: &shuck_ast::ZshQualifiedGlob,
) {
    for segment in &glob.segments {
        match segment {
            ZshGlobSegment::Pattern(pattern) => visitor.visit_pattern(pattern),
            ZshGlobSegment::InlineControl(_) => {}
        }
    }
    if let Some(qualifiers) = &glob.qualifiers {
        walk_zsh_glob_qualifier_group(visitor, qualifiers);
    }
}

fn walk_zsh_glob_qualifier_group<V: AstVisitor + ?Sized>(
    visitor: &mut V,
    group: &ZshGlobQualifierGroup,
) {
    for fragment in &group.fragments {
        match fragment {
            ZshGlobQualifier::LetterSequence { text, .. } => visitor.visit_source_text(text),
            ZshGlobQualifier::NumericArgument { start, end, .. } => {
                visitor.visit_source_text(start);
                if let Some(end) = end {
                    visitor.visit_source_text(end);
                }
            }
            ZshGlobQualifier::Negation { .. } | ZshGlobQualifier::Flag { .. } => {}
        }
    }
}
