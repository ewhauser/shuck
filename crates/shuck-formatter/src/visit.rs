use shuck_ast::{
    AnonymousFunctionCommand, ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, Assignment,
    AssignmentValue, BourneParameterExpansion, Command, CompoundCommand, ConditionalExpr,
    DeclClause, DeclOperand, File, FunctionDef, Heredoc, HeredocBody, HeredocBodyPart,
    HeredocBodyPartNode, ParameterExpansion, ParameterExpansionSyntax, ParameterOp, Pattern,
    PatternPart, PatternPartNode, Redirect, RedirectTarget, SourceText, Stmt, StmtSeq, Subscript,
    VarRef, Word, WordPart, WordPartNode, ZshExpansionOperation, ZshExpansionTarget,
    ZshGlobQualifier, ZshGlobQualifierGroup, ZshGlobSegment, ZshParameterExpansion,
};

use crate::command::{
    array_elem_parts, array_elem_value_word_mut, builtin_like_parts, builtin_like_parts_mut,
};

pub(crate) trait AstVisitor {
    fn visit_stmt_seq(&mut self, sequence: &StmtSeq) {
        walk_stmt_seq(self, sequence);
    }

    fn visit_stmt(&mut self, stmt: &Stmt) {
        walk_stmt(self, stmt);
    }

    fn visit_command(&mut self, command: &Command) {
        walk_command(self, command);
    }

    fn visit_compound_command(&mut self, command: &CompoundCommand) {
        walk_compound_command(self, command);
    }

    fn visit_function(&mut self, function: &FunctionDef) {
        walk_function(self, function);
    }

    fn visit_anonymous_function(&mut self, function: &AnonymousFunctionCommand) {
        walk_anonymous_function(self, function);
    }

    fn visit_redirect(&mut self, redirect: &Redirect) {
        walk_redirect(self, redirect);
    }

    fn visit_assignment(&mut self, assignment: &Assignment) {
        walk_assignment(self, assignment);
    }

    fn visit_var_ref(&mut self, reference: &VarRef) {
        walk_var_ref(self, reference);
    }

    fn visit_subscript(&mut self, subscript: &Subscript) {
        walk_subscript(self, subscript);
    }

    fn visit_conditional_expr(&mut self, expression: &ConditionalExpr) {
        walk_conditional_expr(self, expression);
    }

    fn visit_pattern(&mut self, pattern: &Pattern) {
        walk_pattern(self, pattern);
    }

    fn visit_pattern_part(&mut self, part: &PatternPartNode) {
        walk_pattern_part(self, part);
    }

    fn visit_word(&mut self, word: &Word) {
        walk_word(self, word);
    }

    fn visit_word_part(&mut self, part: &WordPartNode) {
        walk_word_part(self, part);
    }

    fn visit_heredoc(&mut self, heredoc: &Heredoc) {
        walk_heredoc(self, heredoc);
    }

    fn visit_heredoc_body(&mut self, body: &HeredocBody) {
        walk_heredoc_body(self, body);
    }

    fn visit_heredoc_body_part(&mut self, part: &HeredocBodyPartNode) {
        walk_heredoc_body_part(self, part);
    }

    fn visit_arithmetic_expr(&mut self, expression: &ArithmeticExprNode) {
        walk_arithmetic_expr(self, expression);
    }

    fn visit_arithmetic_lvalue(&mut self, target: &ArithmeticLvalue) {
        walk_arithmetic_lvalue(self, target);
    }

    fn visit_parameter_expansion(&mut self, parameter: &ParameterExpansion) {
        walk_parameter_expansion(self, parameter);
    }

    fn visit_parameter_op(&mut self, operator: &ParameterOp) {
        walk_parameter_op(self, operator);
    }

    fn visit_source_text(&mut self, _text: &SourceText) {}
}

pub(crate) trait AstVisitorMut {
    fn visit_file(&mut self, file: &mut File) -> usize {
        walk_file_mut(self, file)
    }

    fn visit_stmt_seq(&mut self, sequence: &mut StmtSeq) -> usize {
        walk_stmt_seq_mut(self, sequence)
    }

    fn visit_stmt(&mut self, stmt: &mut Stmt) -> usize {
        self.enter_stmt(stmt) + walk_stmt_mut(self, stmt)
    }

    fn enter_stmt(&mut self, _stmt: &mut Stmt) -> usize {
        0
    }

    fn visit_command(&mut self, command: &mut Command) -> usize {
        walk_command_mut(self, command)
    }

    fn visit_compound_command(&mut self, command: &mut CompoundCommand) -> usize {
        walk_compound_command_mut(self, command)
    }

    fn visit_function(&mut self, function: &mut FunctionDef) -> usize {
        walk_function_mut(self, function)
    }

    fn visit_anonymous_function(&mut self, function: &mut AnonymousFunctionCommand) -> usize {
        walk_anonymous_function_mut(self, function)
    }

    fn visit_redirect(&mut self, redirect: &mut Redirect) -> usize {
        walk_redirect_mut(self, redirect)
    }

    fn visit_assignment(&mut self, assignment: &mut Assignment) -> usize {
        walk_assignment_mut(self, assignment)
    }

    fn visit_var_ref(&mut self, reference: &mut VarRef) -> usize {
        walk_var_ref_mut(self, reference)
    }

    fn visit_subscript(&mut self, subscript: &mut Subscript) -> usize {
        walk_subscript_mut(self, subscript)
    }

    fn visit_conditional_expr(&mut self, expression: &mut ConditionalExpr) -> usize {
        walk_conditional_expr_mut(self, expression)
    }

    fn visit_pattern(&mut self, pattern: &mut Pattern) -> usize {
        walk_pattern_mut(self, pattern)
    }

    fn visit_pattern_part(&mut self, part: &mut PatternPartNode) -> usize {
        walk_pattern_part_mut(self, part)
    }

    fn visit_word(&mut self, word: &mut Word) -> usize {
        let changes = walk_word_mut(self, word);
        changes + self.leave_word(word)
    }

    fn leave_word(&mut self, _word: &mut Word) -> usize {
        0
    }

    fn visit_word_part(&mut self, part: &mut WordPartNode) -> usize {
        walk_word_part_mut(self, part)
    }

    fn visit_heredoc(&mut self, heredoc: &mut Heredoc) -> usize {
        walk_heredoc_mut(self, heredoc)
    }

    fn visit_heredoc_body(&mut self, body: &mut HeredocBody) -> usize {
        walk_heredoc_body_mut(self, body)
    }

    fn visit_heredoc_body_part(&mut self, part: &mut HeredocBodyPartNode) -> usize {
        walk_heredoc_body_part_mut(self, part)
    }

    fn visit_arithmetic_expr(&mut self, expression: &mut ArithmeticExprNode) -> usize {
        walk_arithmetic_expr_mut(self, expression)
    }

    fn visit_arithmetic_lvalue(&mut self, target: &mut ArithmeticLvalue) -> usize {
        walk_arithmetic_lvalue_mut(self, target)
    }

    fn visit_parameter_expansion(&mut self, parameter: &mut ParameterExpansion) -> usize {
        walk_parameter_expansion_mut(self, parameter)
    }

    fn visit_parameter_op(&mut self, operator: &mut ParameterOp) -> usize {
        walk_parameter_op_mut(self, operator)
    }

    fn visit_source_text(&mut self, _text: &mut SourceText) -> usize {
        0
    }
}

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

pub(crate) fn walk_word_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    word: &mut Word,
) -> usize {
    word.parts
        .iter_mut()
        .map(|part| walk_word_part_surface_source_texts_mut(visitor, part))
        .sum()
}

pub(crate) fn walk_word_part_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    part: &mut WordPartNode,
) -> usize {
    match &mut part.kind {
        WordPart::ZshQualifiedGlob(glob) => {
            walk_zsh_qualified_glob_surface_source_texts_mut(visitor, glob)
        }
        WordPart::SingleQuoted { value, .. } => visitor.visit_source_text(value),
        WordPart::DoubleQuoted { parts, .. } => parts
            .iter_mut()
            .map(|part| walk_word_part_surface_source_texts_mut(visitor, part))
            .sum(),
        WordPart::ArithmeticExpansion { expression, .. } => visitor.visit_source_text(expression),
        WordPart::Parameter(parameter) => visitor.visit_parameter_expansion(parameter),
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            ..
        } => {
            let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference);
            if let Some(operand) = operand {
                changes += visitor.visit_source_text(operand);
            }
            changes + walk_parameter_op_surface_source_texts_mut(visitor, operator)
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => {
            walk_var_ref_surface_source_texts_mut(visitor, reference)
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
            let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference)
                + visitor.visit_source_text(offset);
            if let Some(length) = length {
                changes += visitor.visit_source_text(length);
            }
            changes
        }
        WordPart::IndirectExpansion {
            reference,
            operand,
            operator,
            ..
        } => {
            let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference);
            if let Some(operand) = operand {
                changes += visitor.visit_source_text(operand);
            }
            if let Some(operator) = operator {
                changes += walk_parameter_op_surface_source_texts_mut(visitor, operator);
            }
            changes
        }
        WordPart::Literal(_)
        | WordPart::Variable(_)
        | WordPart::CommandSubstitution { .. }
        | WordPart::ProcessSubstitution { .. }
        | WordPart::PrefixMatch { .. } => 0,
    }
}

fn walk_var_ref_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    reference: &mut VarRef,
) -> usize {
    reference.subscript.as_mut().map_or(0, |subscript| {
        walk_subscript_surface_source_texts_mut(visitor, subscript)
    })
}

fn walk_subscript_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    subscript: &mut Subscript,
) -> usize {
    let mut changes = visitor.visit_source_text(&mut subscript.text);
    if let Some(raw) = &mut subscript.raw {
        changes += visitor.visit_source_text(raw);
    }
    changes
}

fn walk_pattern_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    pattern: &mut Pattern,
) -> usize {
    pattern
        .parts
        .iter_mut()
        .map(|part| match &mut part.kind {
            PatternPart::CharClass(text) => visitor.visit_source_text(text),
            PatternPart::Group { patterns, .. } => patterns
                .iter_mut()
                .map(|pattern| walk_pattern_surface_source_texts_mut(visitor, pattern))
                .sum(),
            PatternPart::Word(word) => walk_word_surface_source_texts_mut(visitor, word),
            PatternPart::Literal(_) | PatternPart::AnyString | PatternPart::AnyChar => 0,
        })
        .sum()
}

pub(crate) fn walk_parameter_expansion_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    parameter: &mut ParameterExpansion,
) -> usize {
    match &mut parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                walk_var_ref_surface_source_texts_mut(visitor, reference)
            }
            BourneParameterExpansion::Indirect {
                reference,
                operator,
                operand,
                ..
            } => {
                let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference);
                if let Some(operand) = operand {
                    changes += visitor.visit_source_text(operand);
                }
                if let Some(operator) = operator {
                    changes += walk_parameter_op_surface_source_texts_mut(visitor, operator);
                }
                changes
            }
            BourneParameterExpansion::PrefixMatch { .. } => 0,
            BourneParameterExpansion::Slice {
                reference,
                offset,
                length,
                ..
            } => {
                let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference)
                    + visitor.visit_source_text(offset);
                if let Some(length) = length {
                    changes += visitor.visit_source_text(length);
                }
                changes
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                ..
            } => {
                let mut changes = walk_var_ref_surface_source_texts_mut(visitor, reference);
                if let Some(operand) = operand {
                    changes += visitor.visit_source_text(operand);
                }
                changes + walk_parameter_op_surface_source_texts_mut(visitor, operator)
            }
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            walk_zsh_parameter_expansion_surface_source_texts_mut(visitor, syntax)
        }
    }
}

fn walk_zsh_parameter_expansion_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    syntax: &mut ZshParameterExpansion,
) -> usize {
    match &mut syntax.target {
        ZshExpansionTarget::Reference(reference) => {
            walk_var_ref_surface_source_texts_mut(visitor, reference)
        }
        ZshExpansionTarget::Word(word) => walk_word_surface_source_texts_mut(visitor, word),
        ZshExpansionTarget::Nested(parameter) => visitor.visit_parameter_expansion(parameter),
        ZshExpansionTarget::Empty => 0,
    }
}

fn walk_parameter_op_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    operator: &mut ParameterOp,
) -> usize {
    match operator {
        ParameterOp::RemovePrefixShort { pattern }
        | ParameterOp::RemovePrefixLong { pattern }
        | ParameterOp::RemoveSuffixShort { pattern }
        | ParameterOp::RemoveSuffixLong { pattern } => {
            walk_pattern_surface_source_texts_mut(visitor, pattern)
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
        } => {
            walk_pattern_surface_source_texts_mut(visitor, pattern)
                + visitor.visit_source_text(replacement)
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

fn walk_zsh_qualified_glob_surface_source_texts_mut<V: AstVisitorMut + ?Sized>(
    visitor: &mut V,
    glob: &mut shuck_ast::ZshQualifiedGlob,
) -> usize {
    let mut changes = 0;
    for segment in &mut glob.segments {
        changes += match segment {
            ZshGlobSegment::Pattern(pattern) => {
                walk_pattern_surface_source_texts_mut(visitor, pattern)
            }
            ZshGlobSegment::InlineControl(_) => 0,
        };
    }
    if let Some(qualifiers) = &mut glob.qualifiers {
        changes += walk_zsh_glob_qualifier_group_mut(visitor, qualifiers);
    }
    changes
}
