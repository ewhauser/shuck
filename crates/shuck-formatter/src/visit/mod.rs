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

// Keep the stable `crate::visit` surface here while the walkers live by traversal mode.
mod immutable;
mod mutable;
mod surface_text;

pub(crate) use immutable::*;
pub(crate) use mutable::*;
pub(crate) use surface_text::*;
