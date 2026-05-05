use super::inert::{
    heredoc_body_is_semantically_inert, pattern_is_semantically_inert, word_is_semantically_inert,
    word_part_is_semantically_inert, zsh_qualified_glob_is_semantically_inert,
};
use super::*;

impl<'a, 'observer> SemanticModelBuilder<'a, 'observer> {
    pub(super) fn visit_words(
        &mut self,
        words: &'a [Word],
        kind: WordVisitKind,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_words_into(words, kind, flow, &mut nested_regions);
        nested_regions
    }

    pub(super) fn visit_words_into(
        &mut self,
        words: &'a [Word],
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for word in words {
            self.visit_word_into(word, kind, flow, nested_regions);
        }
    }

    pub(super) fn visit_array_expr_into(
        &mut self,
        array: &'a ArrayExpr,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for element in &array.elements {
            match element {
                ArrayElem::Sequential(word) => {
                    self.visit_word_into(word, kind, flow, nested_regions)
                }
                ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                    self.visit_var_ref_subscript_words(None, Some(key), kind, flow, nested_regions);
                    self.visit_word_into(value, kind, flow, nested_regions);
                }
            }
        }
    }

    pub(super) fn visit_patterns(
        &mut self,
        patterns: &'a [Pattern],
        kind: WordVisitKind,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_patterns_into(patterns, kind, flow, &mut nested_regions);
        nested_regions
    }

    pub(super) fn visit_patterns_into(
        &mut self,
        patterns: &'a [Pattern],
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for pattern in patterns {
            self.visit_pattern_into(pattern, kind, flow, nested_regions);
        }
    }

    pub(super) fn visit_redirects(
        &mut self,
        redirects: &'a [shuck_ast::Redirect],
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_redirects_into(redirects, flow, &mut nested_regions);
        nested_regions
    }

    pub(super) fn visit_redirects_into(
        &mut self,
        redirects: &'a [shuck_ast::Redirect],
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for redirect in redirects {
            match redirect.word_target() {
                Some(word) => {
                    self.visit_word_into(word, WordVisitKind::Expansion, flow, nested_regions);
                    self.add_redirect_fd_var_binding(redirect);
                }
                None => {
                    self.add_redirect_fd_var_binding(redirect);
                    let Some(heredoc) = redirect.heredoc() else {
                        continue;
                    };
                    if heredoc.delimiter.expands_body {
                        self.visit_heredoc_body_into(
                            &heredoc.body,
                            WordVisitKind::Expansion,
                            flow,
                            nested_regions,
                        );
                    }
                }
            }
        }
    }

    pub(super) fn add_redirect_fd_var_binding(&mut self, redirect: &shuck_ast::Redirect) {
        if redirect.word_target().is_some_and(|target| {
            matches!(
                redirect.kind,
                shuck_ast::RedirectKind::DupInput | shuck_ast::RedirectKind::DupOutput
            ) && shuck_ast::static_word_text(target, self.source).as_deref() == Some("-")
        }) {
            return;
        }

        if let (Some(name), Some(span)) = (&redirect.fd_var, redirect.fd_var_span) {
            self.add_binding(
                name,
                BindingKind::Assignment,
                self.current_scope(),
                span,
                BindingOrigin::Assignment {
                    definition_span: span,
                    value: AssignmentValueOrigin::StaticLiteral,
                },
                BindingAttributes::INTEGER,
            );
        }
    }

    pub(super) fn visit_word(
        &mut self,
        word: &'a Word,
        kind: WordVisitKind,
        flow: FlowState,
    ) -> Vec<IsolatedRegion> {
        let mut nested_regions = Vec::new();
        self.visit_word_into(word, kind, flow, &mut nested_regions);
        nested_regions
    }

    pub(super) fn visit_word_into(
        &mut self,
        word: &'a Word,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if word_is_semantically_inert(word) {
            return;
        }
        self.visit_word_part_nodes(&word.parts, word.span, kind, flow, nested_regions);
    }

    pub(super) fn visit_heredoc_body_into(
        &mut self,
        body: &'a HeredocBody,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if !body.mode.expands() || heredoc_body_is_semantically_inert(body, self.source) {
            return;
        }
        self.visit_heredoc_body_part_nodes(&body.parts, kind, flow, nested_regions);
    }

    pub(super) fn visit_pattern_into(
        &mut self,
        pattern: &'a Pattern,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        if pattern_is_semantically_inert(pattern) {
            return;
        }
        self.visit_pattern_part_nodes(&pattern.parts, kind, flow, nested_regions);
    }

    pub(super) fn visit_word_part_nodes(
        &mut self,
        parts: &'a [WordPartNode],
        word_span: Span,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let escaped_template_starts =
            escaped_parameter_template_body_starts(word_span, self.source);
        for part in parts {
            if !escaped_template_starts.is_empty()
                && escaped_template_starts.contains(&part.span.start.offset)
            {
                continue;
            }
            self.visit_word_part(&part.kind, part.span, kind, flow, nested_regions);
        }
    }

    pub(super) fn visit_pattern_part_nodes(
        &mut self,
        parts: &'a [PatternPartNode],
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for part in parts {
            self.visit_pattern_part(&part.kind, kind, flow, nested_regions);
        }
    }

    pub(super) fn visit_heredoc_body_part_nodes(
        &mut self,
        parts: &'a [HeredocBodyPartNode],
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        for part in parts {
            self.visit_heredoc_body_part(&part.kind, part.span, kind, flow, nested_regions);
        }
    }

    pub(super) fn visit_var_ref_reference(
        &mut self,
        reference: &'a VarRef,
        reference_kind: ReferenceKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
        span: Span,
    ) -> ReferenceId {
        let reference_kind = self.word_reference_kind_override.unwrap_or(reference_kind);
        let id = self.add_reference(&reference.name, reference_kind, span);
        self.visit_var_ref_subscript_words(
            Some(&reference.name),
            reference.subscript.as_deref(),
            word_visit_kind_for_reference_kind(reference_kind),
            flow,
            nested_regions,
        );
        id
    }

    pub(super) fn visit_var_ref_subscript_words(
        &mut self,
        owner_name: Option<&Name>,
        subscript: Option<&'a Subscript>,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let Some(subscript) = subscript else {
            return;
        };
        if subscript.selector().is_some() {
            return;
        }
        let uses_associative_word_semantics = matches!(
            subscript.interpretation,
            shuck_ast::SubscriptInterpretation::Associative
        ) || owner_name.is_some_and(|name| {
            self.name_uses_associative_word_semantics(name, subscript.span().start.offset)
        });
        if !uses_associative_word_semantics
            && let Some(expression) = subscript.arithmetic_ast.as_ref()
        {
            self.visit_optional_arithmetic_expr_into(Some(expression), flow, nested_regions);
            return;
        }

        if !uses_associative_word_semantics {
            self.visit_unparsed_arithmetic_subscript_references(subscript);
        }

        self.visit_fragment_word(
            subscript.word_ast(),
            Some(subscript.syntax_source_text()),
            kind,
            flow,
            nested_regions,
        );
    }

    pub(super) fn visit_unparsed_arithmetic_subscript_references(&mut self, subscript: &Subscript) {
        for (name, span) in unparsed_arithmetic_subscript_reference_names(
            subscript.syntax_source_text(),
            self.source,
        ) {
            self.add_reference(&name, ReferenceKind::ArithmeticRead, span);
        }
    }

    pub(super) fn visit_word_part(
        &mut self,
        part: &'a WordPart,
        span: Span,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match part {
            WordPart::ZshQualifiedGlob(glob) => {
                if zsh_qualified_glob_is_semantically_inert(glob) {
                    return;
                }
                for segment in &glob.segments {
                    if let ZshGlobSegment::Pattern(pattern) = segment {
                        self.visit_pattern_into(pattern, kind, flow, nested_regions);
                    }
                }
            }
            WordPart::Literal(_) | WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => {
                if parts
                    .iter()
                    .all(|part| word_part_is_semantically_inert(&part.kind))
                {
                    return;
                }
                self.visit_word_part_nodes(parts, span, kind, flow, nested_regions);
            }
            WordPart::Variable(name) => {
                if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh
                    && is_zsh_parameter_existence_name(name)
                {
                    return;
                }
                self.add_reference(
                    name,
                    self.word_reference_kind_override
                        .unwrap_or(reference_kind_for_word_visit(
                            kind,
                            ReferenceKind::Expansion,
                        )),
                    span,
                );
            }
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => {
                let scope =
                    self.push_scope(ScopeKind::CommandSubstitution, self.current_scope(), span);
                let mut commands = Vec::with_capacity(body.len());
                self.visit_stmt_seq_into(
                    body,
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                    &mut commands,
                );
                self.pop_scope(scope);
                self.mark_scope_completed(scope);
                nested_regions.push(IsolatedRegion {
                    scope,
                    commands: self.recorded_program.push_command_ids(commands),
                });
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                self.visit_optional_arithmetic_expr_into(
                    expression_ast.as_deref(),
                    flow,
                    nested_regions,
                );
            }
            WordPart::Parameter(parameter) => {
                self.visit_parameter_expansion(
                    parameter,
                    kind,
                    flow,
                    nested_regions,
                    parameter.span,
                );
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                let reference_id = self.visit_var_ref_reference(
                    reference,
                    parameter_operation_reference_kind(kind, operator),
                    flow,
                    nested_regions,
                    reference.span,
                );
                if parameter_operator_guards_unset_reference(operator) {
                    self.record_guarded_parameter_reference(reference_id);
                }
                if matches!(operator.as_ref(), ParameterOp::AssignDefault) {
                    self.add_parameter_default_binding(reference);
                }
                self.visit_parameter_operator_operand(
                    operator,
                    operand.as_ref(),
                    operand_word_ast.as_deref(),
                    kind,
                    flow,
                    nested_regions,
                );
            }
            WordPart::Length(reference) | WordPart::ArrayLength(reference) => {
                self.visit_var_ref_reference(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::Length),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPart::ArrayAccess(reference) => {
                if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh
                    && is_zsh_parameter_existence_name(&reference.name)
                {
                    let owner_name = Name::from(
                        reference
                            .name
                            .as_str()
                            .strip_prefix('+')
                            .expect("zsh existence-test names start with plus"),
                    );
                    self.visit_zsh_existence_probe_subscript(
                        &owner_name,
                        reference.subscript.as_deref(),
                        kind,
                        flow,
                        nested_regions,
                    );
                    return;
                }
                self.visit_var_ref_reference(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::ArrayAccess),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPart::ArrayIndices(reference) => {
                self.visit_var_ref_reference(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
            WordPart::PrefixMatch { .. } => {}
            WordPart::IndirectExpansion {
                reference,
                operator,
                operand,
                operand_word_ast,
                ..
            } => {
                let id = self.visit_var_ref_reference(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
                    flow,
                    nested_regions,
                    reference.span,
                );
                self.indirect_expansion_refs.insert(id);
                if let Some(operator) = operator {
                    self.visit_parameter_operator_operand(
                        operator,
                        operand.as_ref(),
                        operand_word_ast.as_deref(),
                        kind,
                        flow,
                        nested_regions,
                    );
                }
            }
            WordPart::Substring {
                reference,
                offset_ast,
                length_ast,
                ..
            } => {
                self.visit_var_ref_reference(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                    flow,
                    nested_regions,
                    reference.span,
                );
                self.visit_optional_arithmetic_expr_into(
                    offset_ast.as_deref(),
                    flow,
                    nested_regions,
                );
                self.visit_optional_arithmetic_expr_into(
                    length_ast.as_deref(),
                    flow,
                    nested_regions,
                );
            }
            WordPart::ArraySlice {
                reference,
                offset_ast,
                length_ast,
                ..
            } => {
                self.visit_var_ref_reference(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                    flow,
                    nested_regions,
                    reference.span,
                );
                self.visit_optional_arithmetic_expr_into(
                    offset_ast.as_deref(),
                    flow,
                    nested_regions,
                );
                self.visit_optional_arithmetic_expr_into(
                    length_ast.as_deref(),
                    flow,
                    nested_regions,
                );
            }
            WordPart::Transformation { reference, .. } => {
                self.visit_var_ref_reference(
                    reference,
                    reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                    flow,
                    nested_regions,
                    reference.span,
                );
            }
        }
    }

    pub(super) fn visit_heredoc_body_part(
        &mut self,
        part: &'a HeredocBodyPart,
        span: Span,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match part {
            HeredocBodyPart::Literal(text) => {
                self.visit_escaped_braced_literal_references(text, span, kind);
            }
            HeredocBodyPart::Variable(name) => {
                self.add_reference(
                    name,
                    reference_kind_for_word_visit(kind, ReferenceKind::Expansion),
                    span,
                );
            }
            HeredocBodyPart::CommandSubstitution { body, .. } => {
                let scope =
                    self.push_scope(ScopeKind::CommandSubstitution, self.current_scope(), span);
                let mut commands = Vec::with_capacity(body.len());
                self.visit_stmt_seq_into(
                    body,
                    FlowState {
                        in_subshell: true,
                        ..flow
                    },
                    &mut commands,
                );
                self.pop_scope(scope);
                self.mark_scope_completed(scope);
                nested_regions.push(IsolatedRegion {
                    scope,
                    commands: self.recorded_program.push_command_ids(commands),
                });
            }
            HeredocBodyPart::ArithmeticExpansion { expression_ast, .. } => {
                self.visit_optional_arithmetic_expr_into(
                    expression_ast.as_ref(),
                    flow,
                    nested_regions,
                );
            }
            HeredocBodyPart::Parameter(parameter) => {
                self.visit_parameter_expansion(parameter, kind, flow, nested_regions, span);
            }
        }
    }

    pub(super) fn visit_escaped_braced_literal_references(
        &mut self,
        text: &LiteralText,
        span: Span,
        kind: WordVisitKind,
    ) {
        if !text.is_source_backed() {
            return;
        }

        for (name, span) in
            escaped_braced_literal_reference_names(text.syntax_str(self.source, span), span)
        {
            self.add_reference(
                &name,
                reference_kind_for_word_visit(kind, ReferenceKind::Expansion),
                span,
            );
        }
    }

    pub(super) fn visit_parameter_expansion(
        &mut self,
        parameter: &'a ParameterExpansion,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
        span: Span,
    ) {
        if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh
            && zsh_parameter_expansion_is_existence_test(parameter, self.source)
        {
            self.visit_zsh_parameter_existence_probe_subscript(parameter, kind);
            return;
        }

        match &parameter.syntax {
            ParameterExpansionSyntax::Bourne(syntax) => match syntax {
                BourneParameterExpansion::Access { reference } => {
                    self.visit_var_ref_reference(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::ArrayAccess),
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansion::Length { reference } => {
                    self.visit_var_ref_reference(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::Length),
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansion::Indices { reference } => {
                    self.visit_var_ref_reference(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
                        flow,
                        nested_regions,
                        span,
                    );
                }
                BourneParameterExpansion::Indirect {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    let id = self.visit_var_ref_reference(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::IndirectExpansion),
                        flow,
                        nested_regions,
                        span,
                    );
                    self.indirect_expansion_refs.insert(id);
                    if let Some(operator) = operator {
                        self.visit_parameter_operator_operand(
                            operator,
                            operand.as_ref(),
                            operand_word_ast.as_deref(),
                            kind,
                            flow,
                            nested_regions,
                        );
                    }
                }
                BourneParameterExpansion::PrefixMatch { .. } => {}
                BourneParameterExpansion::Slice {
                    reference,
                    offset_ast,
                    length_ast,
                    ..
                } => {
                    self.visit_var_ref_reference(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                        flow,
                        nested_regions,
                        span,
                    );
                    self.visit_parameter_slice_arithmetic_expr_into(
                        offset_ast.as_deref(),
                        flow,
                        nested_regions,
                    );
                    self.visit_parameter_slice_arithmetic_expr_into(
                        length_ast.as_deref(),
                        flow,
                        nested_regions,
                    );
                }
                BourneParameterExpansion::Operation {
                    reference,
                    operator,
                    operand,
                    operand_word_ast,
                    ..
                } => {
                    let reference_id = self.visit_var_ref_reference(
                        reference,
                        parameter_operation_reference_kind(kind, operator),
                        flow,
                        nested_regions,
                        span,
                    );
                    if parameter_operator_guards_unset_reference(operator) {
                        self.record_guarded_parameter_reference(reference_id);
                    }
                    if matches!(operator.as_ref(), ParameterOp::AssignDefault) {
                        self.add_parameter_default_binding(reference);
                    }
                    self.visit_parameter_operator_operand(
                        operator,
                        operand.as_ref(),
                        operand_word_ast.as_deref(),
                        kind,
                        flow,
                        nested_regions,
                    );
                }
                BourneParameterExpansion::Transformation { reference, .. } => {
                    self.visit_var_ref_reference(
                        reference,
                        reference_kind_for_word_visit(kind, ReferenceKind::ParameterExpansion),
                        flow,
                        nested_regions,
                        span,
                    );
                }
            },
            ParameterExpansionSyntax::Zsh(syntax) => {
                match &syntax.target {
                    ZshExpansionTarget::Reference(reference) => {
                        if self.shell_profile.dialect == shuck_parser::ShellDialect::Zsh {
                            self.visit_var_ref_reference(
                                reference,
                                reference_kind_for_word_visit(
                                    kind,
                                    ReferenceKind::ParameterExpansion,
                                ),
                                flow,
                                nested_regions,
                                span,
                            );
                        }
                    }
                    ZshExpansionTarget::Word(word) => {
                        self.visit_word_into(word, kind, flow, nested_regions);
                    }
                    ZshExpansionTarget::Nested(parameter) => {
                        self.visit_parameter_expansion(parameter, kind, flow, nested_regions, span);
                    }
                    ZshExpansionTarget::Empty => {}
                }

                for modifier in &syntax.modifiers {
                    self.visit_fragment_word(
                        modifier.argument_word_ast(),
                        modifier.argument.as_ref(),
                        kind,
                        flow,
                        nested_regions,
                    );
                }

                if let Some(operation) = &syntax.operation {
                    match operation {
                        ZshExpansionOperation::PatternOperation { operand, .. }
                        | ZshExpansionOperation::TrimOperation { operand, .. } => self
                            .visit_fragment_word(
                                operation.operand_word_ast(),
                                Some(operand),
                                kind,
                                flow,
                                nested_regions,
                            ),
                        ZshExpansionOperation::Defaulting { operand, .. } => {
                            self.guarded_parameter_operand_depth += 1;
                            self.defaulting_parameter_operand_depth += 1;
                            self.visit_fragment_word(
                                operation.operand_word_ast(),
                                Some(operand),
                                kind,
                                flow,
                                nested_regions,
                            );
                            self.guarded_parameter_operand_depth -= 1;
                            self.defaulting_parameter_operand_depth -= 1;
                        }
                        ZshExpansionOperation::ReplacementOperation {
                            pattern,
                            replacement,
                            ..
                        } => {
                            self.visit_fragment_word(
                                operation.pattern_word_ast(),
                                Some(pattern),
                                WordVisitKind::ParameterPattern,
                                flow,
                                nested_regions,
                            );
                            self.visit_fragment_word(
                                operation.replacement_word_ast(),
                                replacement.as_ref(),
                                kind,
                                flow,
                                nested_regions,
                            );
                        }
                        ZshExpansionOperation::Slice { offset, length, .. } => {
                            self.visit_fragment_word(
                                operation.offset_word_ast(),
                                Some(offset),
                                kind,
                                flow,
                                nested_regions,
                            );
                            self.visit_fragment_word(
                                operation.length_word_ast(),
                                length.as_ref(),
                                kind,
                                flow,
                                nested_regions,
                            );
                        }
                        ZshExpansionOperation::Unknown { text, .. } => self.visit_fragment_word(
                            operation.operand_word_ast(),
                            Some(text),
                            kind,
                            flow,
                            nested_regions,
                        ),
                    }
                }
            }
        }
    }

    fn visit_zsh_existence_probe_subscript(
        &mut self,
        owner_name: &Name,
        subscript: Option<&'a Subscript>,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        self.visit_var_ref_subscript_words(Some(owner_name), subscript, kind, flow, nested_regions);
    }

    fn visit_zsh_parameter_existence_probe_subscript(
        &mut self,
        parameter: &ParameterExpansion,
        kind: WordVisitKind,
    ) {
        let Some((subscript_text, subscript_span)) = zsh_parameter_existence_subscript_text(
            parameter.raw_body.slice(self.source),
            parameter,
        ) else {
            return;
        };
        let source_offsets = (0..subscript_text.len()).collect::<Vec<_>>();
        for (name, span) in scan_parameter_reference_names(
            subscript_text,
            subscript_text,
            &source_offsets,
            subscript_span,
        ) {
            self.add_reference(
                &name,
                reference_kind_for_word_visit(kind, ReferenceKind::Expansion),
                span,
            );
        }
    }

    pub(super) fn visit_fragment_word(
        &mut self,
        word: Option<&'a Word>,
        text: Option<&shuck_ast::SourceText>,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        let Some(word) = word else {
            debug_assert!(
                text.is_none(),
                "parser-backed fragment text should always carry a word AST"
            );
            return;
        };
        self.visit_word_into(word, kind, flow, nested_regions);
    }

    pub(super) fn visit_parameter_operator_operand(
        &mut self,
        operator: &'a ParameterOp,
        operand: Option<&shuck_ast::SourceText>,
        operand_word_ast: Option<&'a Word>,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match operator {
            ParameterOp::RemovePrefixShort { pattern }
            | ParameterOp::RemovePrefixLong { pattern }
            | ParameterOp::RemoveSuffixShort { pattern }
            | ParameterOp::RemoveSuffixLong { pattern } => {
                self.visit_pattern_into(
                    pattern,
                    WordVisitKind::ParameterPattern,
                    flow,
                    nested_regions,
                );
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
                self.visit_pattern_into(
                    pattern,
                    WordVisitKind::ParameterPattern,
                    flow,
                    nested_regions,
                );
                self.visit_fragment_word(
                    operator.replacement_word_ast(),
                    Some(replacement),
                    kind,
                    flow,
                    nested_regions,
                );
            }
            ParameterOp::UseDefault | ParameterOp::UseReplacement => {
                self.guarded_parameter_operand_depth += 1;
                self.defaulting_parameter_operand_depth += 1;
                self.visit_fragment_word(operand_word_ast, operand, kind, flow, nested_regions);
                self.guarded_parameter_operand_depth -= 1;
                self.defaulting_parameter_operand_depth -= 1;
            }
            ParameterOp::AssignDefault | ParameterOp::Error => {
                self.defaulting_parameter_operand_depth += 1;
                self.visit_fragment_word(operand_word_ast, operand, kind, flow, nested_regions);
                self.defaulting_parameter_operand_depth -= 1;
            }
            ParameterOp::UpperFirst
            | ParameterOp::UpperAll
            | ParameterOp::LowerFirst
            | ParameterOp::LowerAll => {}
        }
    }

    pub(super) fn record_guarded_parameter_reference(&mut self, reference_id: ReferenceId) {
        self.guarded_parameter_refs.insert(reference_id);
        if self.defaulting_parameter_operand_depth == 0 && self.short_circuit_condition_depth == 0 {
            self.parameter_guard_flow_refs.insert(reference_id);
        }
    }

    pub(super) fn visit_pattern_part(
        &mut self,
        part: &'a PatternPart,
        kind: WordVisitKind,
        flow: FlowState,
        nested_regions: &mut Vec<IsolatedRegion>,
    ) {
        match part {
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    self.visit_pattern_into(pattern, kind, flow, nested_regions);
                }
            }
            PatternPart::Word(word) => {
                self.visit_word_into(word, kind, flow, nested_regions);
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn is_zsh_parameter_existence_name(name: &Name) -> bool {
    name.as_str()
        .strip_prefix('+')
        .is_some_and(is_shell_identifier)
}

fn zsh_parameter_expansion_is_existence_test(parameter: &ParameterExpansion, source: &str) -> bool {
    parameter.raw_body.is_source_backed()
        && zsh_parameter_body_is_existence_test(parameter.raw_body.slice(source))
}

fn zsh_parameter_body_is_existence_test(body: &str) -> bool {
    body.strip_prefix('+').is_some_and(|body| {
        let name_end = body
            .char_indices()
            .find_map(|(index, ch)| (!is_shell_name_char(ch)).then_some(index))
            .unwrap_or(body.len());
        name_end > 0 && is_shell_identifier(&body[..name_end])
    })
}

fn zsh_parameter_existence_subscript_text<'a>(
    raw_body: &'a str,
    parameter: &ParameterExpansion,
) -> Option<(&'a str, Span)> {
    let body = raw_body.strip_prefix('+')?;
    let name_end = body
        .char_indices()
        .find_map(|(index, ch)| (!is_shell_name_char(ch)).then_some(index))
        .unwrap_or(body.len());
    if name_end == 0 || !is_shell_identifier(&body[..name_end]) {
        return None;
    }

    let subscript_open = name_end;
    if body.as_bytes().get(subscript_open) != Some(&b'[') {
        return None;
    }

    let subscript_body_start = subscript_open + 1;
    let mut depth = 1usize;
    for (relative_index, ch) in body[subscript_body_start..].char_indices() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    let subscript_body_end = subscript_body_start + relative_index;
                    let raw_body_span = parameter.raw_body.span();
                    let start_offset = 1 + subscript_body_start;
                    let end_offset = 1 + subscript_body_end;
                    let start = raw_body_span.start.advanced_by(&raw_body[..start_offset]);
                    let end = raw_body_span.start.advanced_by(&raw_body[..end_offset]);
                    return Some((
                        &body[subscript_body_start..subscript_body_end],
                        Span::from_positions(start, end),
                    ));
                }
            }
            _ => {}
        }
    }

    None
}

fn is_shell_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(is_shell_name_start) && chars.all(is_shell_name_char)
}

fn is_shell_name_start(ch: char) -> bool {
    ch == '_' || ch.is_ascii_alphabetic()
}

fn is_shell_name_char(ch: char) -> bool {
    is_shell_name_start(ch) || ch.is_ascii_digit()
}
