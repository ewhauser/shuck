use super::*;

impl<'a, 'observer> SemanticModelBuilder<'a, 'observer> {
    pub(super) fn visit_conditional_expr(
        &mut self,
        expression: &'a ConditionalExpr,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_conditional_expr_into(expression, flow, &mut nested_regions);
        nested_regions
    }

    pub(super) fn visit_conditional_expr_into(
        &mut self,
        expression: &'a ConditionalExpr,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match expression {
            ConditionalExpr::Binary(expr) => {
                if conditional_binary_op_uses_arithmetic_operands(expr.op) {
                    self.visit_conditional_arithmetic_operand_into(
                        &expr.left,
                        flow,
                        nested_regions,
                    );
                    self.visit_conditional_arithmetic_operand_into(
                        &expr.right,
                        flow,
                        nested_regions,
                    );
                } else if matches!(expr.op, ConditionalBinaryOp::And | ConditionalBinaryOp::Or) {
                    self.visit_conditional_expr_into(&expr.left, flow, nested_regions);
                    self.short_circuit_condition_depth += 1;
                    self.visit_conditional_expr_into(&expr.right, flow, nested_regions);
                    self.short_circuit_condition_depth -= 1;
                } else {
                    self.visit_conditional_expr_into(&expr.left, flow, nested_regions);
                    self.visit_conditional_expr_into(&expr.right, flow, nested_regions);
                }
            }
            ConditionalExpr::Unary(expr) => {
                if expr.op == ConditionalUnaryOp::VariableSet
                    && let Some((name, span)) =
                        variable_set_test_operand_name(&expr.expr, self.source)
                {
                    self.add_reference_if_bound(&name, ReferenceKind::ConditionalOperand, span);
                }
                self.visit_conditional_expr_into(&expr.expr, flow, nested_regions);
            }
            ConditionalExpr::Parenthesized(expr) => {
                self.visit_conditional_expr_into(&expr.expr, flow, nested_regions);
            }
            ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
                self.visit_word_into(word, WordVisitKind::Conditional, flow, nested_regions);
            }
            ConditionalExpr::Pattern(pattern) => {
                self.visit_pattern_into(pattern, WordVisitKind::Conditional, flow, nested_regions);
            }
            ConditionalExpr::VarRef(var_ref) => {
                self.visit_var_ref_reference(
                    var_ref,
                    ReferenceKind::ConditionalOperand,
                    flow,
                    nested_regions,
                    var_ref.name_span,
                );
            }
        }
    }

    pub(super) fn visit_conditional_arithmetic_operand_into(
        &mut self,
        expression: &'a ConditionalExpr,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if let Some((name, span)) = conditional_arithmetic_operand_name(expression, self.source) {
            self.add_reference(&name, ReferenceKind::ArithmeticRead, span);
            return;
        }

        self.visit_conditional_expr_into(expression, flow, nested_regions);
    }

    pub(super) fn visit_optional_arithmetic_expr(
        &mut self,
        expr: Option<&'a ArithmeticExprNode>,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_optional_arithmetic_expr_into(expr, flow, &mut nested_regions);
        nested_regions
    }

    pub(super) fn visit_optional_arithmetic_expr_into(
        &mut self,
        expr: Option<&'a ArithmeticExprNode>,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if let Some(expr) = expr {
            self.visit_arithmetic_expr_into(expr, flow, nested_regions);
        }
    }

    pub(super) fn visit_parameter_slice_arithmetic_expr_into(
        &mut self,
        expr: Option<&'a ArithmeticExprNode>,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let previous_kind = self.arithmetic_reference_kind;
        self.arithmetic_reference_kind = ReferenceKind::ParameterSliceArithmetic;
        self.visit_optional_arithmetic_expr_into(expr, flow, nested_regions);
        self.arithmetic_reference_kind = previous_kind;
    }

    pub(super) fn visit_arithmetic_expr_into(
        &mut self,
        expr: &'a ArithmeticExprNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &expr.kind {
            ArithmeticExpr::Number(_) => {}
            ArithmeticExpr::Variable(name) => {
                self.add_reference(name, self.arithmetic_reference_kind, expr.span);
            }
            ArithmeticExpr::Indexed { name, index } => {
                self.add_reference(
                    name,
                    self.arithmetic_reference_kind,
                    arithmetic_name_span(expr.span, name),
                );
                self.visit_arithmetic_index_into(name, index, flow, nested_regions);
            }
            ArithmeticExpr::ShellWord(word) => {
                let previous_kind =
                    if self.arithmetic_reference_kind == ReferenceKind::ParameterSliceArithmetic {
                        self.word_reference_kind_override
                            .replace(ReferenceKind::ParameterSliceArithmetic)
                    } else {
                        None
                    };
                self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions);
                if self.arithmetic_reference_kind == ReferenceKind::ParameterSliceArithmetic {
                    self.word_reference_kind_override = previous_kind;
                }
            }
            ArithmeticExpr::Parenthesized { expression } => {
                self.visit_arithmetic_expr_into(expression, flow, nested_regions);
            }
            ArithmeticExpr::Unary { op, expr: inner } => {
                if matches!(
                    op,
                    ArithmeticUnaryOp::PreIncrement | ArithmeticUnaryOp::PreDecrement
                ) {
                    self.visit_arithmetic_update_into(inner, flow, nested_regions);
                } else {
                    self.visit_arithmetic_expr_into(inner, flow, nested_regions);
                }
            }
            ArithmeticExpr::Postfix { expr: inner, .. } => {
                self.visit_arithmetic_update_into(inner, flow, nested_regions);
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                self.visit_arithmetic_expr_into(left, flow, nested_regions);
                self.visit_arithmetic_expr_into(right, flow, nested_regions);
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_arithmetic_expr_into(condition, flow, nested_regions);
                self.visit_arithmetic_expr_into(then_expr, flow, nested_regions);
                self.visit_arithmetic_expr_into(else_expr, flow, nested_regions);
            }
            ArithmeticExpr::Assignment { target, op, value } => {
                self.visit_arithmetic_assignment_into(
                    target,
                    expr.span,
                    *op,
                    value,
                    flow,
                    nested_regions,
                );
            }
        }
    }

    pub(super) fn visit_arithmetic_index_into(
        &mut self,
        owner_name: &Name,
        index: &'a ArithmeticExprNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if self
            .arithmetic_index_uses_associative_word_semantics(owner_name, index.span.start.offset)
        {
            self.visit_associative_arithmetic_key_into(index, flow, nested_regions);
            return;
        }

        self.visit_arithmetic_expr_into(index, flow, nested_regions);
    }

    pub(super) fn arithmetic_index_uses_associative_word_semantics(
        &self,
        owner_name: &Name,
        offset: usize,
    ) -> bool {
        self.visible_binding_is_assoc(owner_name, offset)
    }

    pub(super) fn visible_binding_is_assoc(&self, name: &Name, offset: usize) -> bool {
        self.resolve_reference(name, self.current_scope(), offset)
            .map(|binding_id| {
                self.bindings[binding_id.index()]
                    .attributes
                    .contains(BindingAttributes::ASSOC)
            })
            .unwrap_or(false)
    }

    pub(super) fn arithmetic_binding_attributes(
        &self,
        target: &ArithmeticLvalue,
        target_offset: usize,
    ) -> BindingAttributes {
        let mut attributes = match target {
            ArithmeticLvalue::Variable(_) => BindingAttributes::empty(),
            ArithmeticLvalue::Indexed { .. } => BindingAttributes::ARRAY,
        };

        if let ArithmeticLvalue::Indexed { name, .. } = target
            && self.visible_binding_is_assoc(name, target_offset)
        {
            attributes |= BindingAttributes::ASSOC;
        }

        attributes
    }

    pub(super) fn visit_associative_arithmetic_key_into(
        &mut self,
        expr: &'a ArithmeticExprNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &expr.kind {
            ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
            ArithmeticExpr::Indexed { index, .. } => {
                self.visit_associative_arithmetic_key_into(index, flow, nested_regions);
            }
            ArithmeticExpr::ShellWord(word) => {
                self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions);
            }
            ArithmeticExpr::Parenthesized { expression } => {
                self.visit_associative_arithmetic_key_into(expression, flow, nested_regions);
            }
            ArithmeticExpr::Unary { expr: inner, .. } => {
                self.visit_associative_arithmetic_key_into(inner, flow, nested_regions);
            }
            ArithmeticExpr::Postfix { expr: inner, .. } => {
                self.visit_associative_arithmetic_key_into(inner, flow, nested_regions);
            }
            ArithmeticExpr::Binary { left, right, .. } => {
                self.visit_associative_arithmetic_key_into(left, flow, nested_regions);
                self.visit_associative_arithmetic_key_into(right, flow, nested_regions);
            }
            ArithmeticExpr::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                self.visit_associative_arithmetic_key_into(condition, flow, nested_regions);
                self.visit_associative_arithmetic_key_into(then_expr, flow, nested_regions);
                self.visit_associative_arithmetic_key_into(else_expr, flow, nested_regions);
            }
            ArithmeticExpr::Assignment { target, value, .. } => {
                self.visit_associative_arithmetic_lvalue_into(target, flow, nested_regions);
                self.visit_associative_arithmetic_key_into(value, flow, nested_regions);
            }
        }
    }

    pub(super) fn visit_associative_arithmetic_lvalue_into(
        &mut self,
        target: &'a ArithmeticLvalue,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match target {
            ArithmeticLvalue::Variable(_) => {}
            ArithmeticLvalue::Indexed { index, .. } => {
                self.visit_associative_arithmetic_key_into(index, flow, nested_regions);
            }
        }
    }

    pub(super) fn visit_arithmetic_update_into(
        &mut self,
        expr: &'a ArithmeticExprNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match &expr.kind {
            ArithmeticExpr::Variable(name) => {
                let reference_id =
                    self.add_reference(name, self.arithmetic_reference_kind, expr.span);
                self.self_referential_assignment_refs.insert(reference_id);
                self.add_binding(
                    name,
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    expr.span,
                    BindingOrigin::ArithmeticAssignment {
                        definition_span: expr.span,
                        target_span: arithmetic_lvalue_span(
                            &ArithmeticLvalue::Variable(name.clone()),
                            expr.span,
                        ),
                    },
                    BindingAttributes::SELF_REFERENTIAL_READ,
                );
            }
            ArithmeticExpr::Indexed { name, index } => {
                self.visit_arithmetic_index_into(name, index, flow, nested_regions);
                let span = arithmetic_name_span(expr.span, name);
                let reference_id = self.add_reference(name, self.arithmetic_reference_kind, span);
                self.self_referential_assignment_refs.insert(reference_id);
                self.add_binding(
                    name,
                    BindingKind::ArithmeticAssignment,
                    self.current_scope(),
                    span,
                    BindingOrigin::ArithmeticAssignment {
                        definition_span: span,
                        target_span: arithmetic_lvalue_span(
                            &ArithmeticLvalue::Indexed {
                                name: name.clone(),
                                index: index.clone(),
                            },
                            expr.span,
                        ),
                    },
                    self.arithmetic_binding_attributes(
                        &ArithmeticLvalue::Indexed {
                            name: name.clone(),
                            index: index.clone(),
                        },
                        span.start.offset,
                    ) | BindingAttributes::SELF_REFERENTIAL_READ,
                );
            }
            _ => {}
        }
    }

    pub(super) fn visit_arithmetic_assignment_into(
        &mut self,
        target: &'a ArithmeticLvalue,
        target_span: Span,
        op: ArithmeticAssignOp,
        value: &'a ArithmeticExprNode,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let name = match target {
            ArithmeticLvalue::Variable(name) | ArithmeticLvalue::Indexed { name, .. } => name,
        };
        let name_span = arithmetic_name_span(target_span, name);
        let reference_start = self.references.len();
        self.visit_arithmetic_lvalue_indices_into(target, flow, nested_regions);
        let mut attributes = self.arithmetic_binding_attributes(target, target_span.start.offset);
        if !matches!(op, ArithmeticAssignOp::Assign) {
            self.add_reference(name, self.arithmetic_reference_kind, name_span);
        }
        self.visit_arithmetic_expr_into(value, flow, nested_regions);
        let self_referential_refs =
            self.newly_added_reference_ids_reading_name(name, reference_start);
        if !self_referential_refs.is_empty() {
            attributes |= BindingAttributes::SELF_REFERENTIAL_READ;
            self.self_referential_assignment_refs
                .extend(self_referential_refs);
        }
        self.add_binding(
            name,
            BindingKind::ArithmeticAssignment,
            self.current_scope(),
            name_span,
            BindingOrigin::ArithmeticAssignment {
                definition_span: name_span,
                target_span: arithmetic_lvalue_span(target, target_span),
            },
            attributes,
        );
    }

    pub(super) fn visit_arithmetic_lvalue_indices_into(
        &mut self,
        target: &'a ArithmeticLvalue,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match target {
            ArithmeticLvalue::Variable(_) => {}
            ArithmeticLvalue::Indexed { name, index } => {
                self.visit_arithmetic_index_into(name, index, flow, nested_regions);
            }
        }
    }
}
