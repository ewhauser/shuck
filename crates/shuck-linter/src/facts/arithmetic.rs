fn collect_zsh_option_map_arithmetic_suppressed_subscripts(
    command: &Command,
    semantic: &SemanticModel,
    command_scope: ScopeId,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Compound(CompoundCommand::Arithmetic(command)) => {
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                command.expr_ast.as_ref(),
                semantic,
                command_scope,
                source,
                spans,
            );
        }
        Command::Compound(CompoundCommand::ArithmeticFor(command)) => {
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                command.init_ast.as_ref(),
                semantic,
                command_scope,
                source,
                spans,
            );
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                command.condition_ast.as_ref(),
                semantic,
                command_scope,
                source,
                spans,
            );
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                command.step_ast.as_ref(),
                semantic,
                command_scope,
                source,
                spans,
            );
        }
        _ => {}
    }
}

fn collect_zsh_option_map_suppressed_subscripts_in_expr(
    expression: Option<&ArithmeticExprNode>,
    semantic: &SemanticModel,
    command_scope: ScopeId,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Some(expression) = expression else {
        return;
    };

    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) | ArithmeticExpr::ShellWord(_) => {}
        ArithmeticExpr::Indexed { name, index } => {
            if arithmetic_index_uses_zsh_option_map_key_semantics(
                semantic,
                command_scope,
                name,
                index,
                source,
            ) {
                spans.push(index.span);
            } else {
                collect_zsh_option_map_suppressed_subscripts_in_expr(
                    Some(index),
                    semantic,
                    command_scope,
                    source,
                    spans,
                );
            }
        }
        ArithmeticExpr::Parenthesized { expression } => {
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                Some(expression),
                semantic,
                command_scope,
                source,
                spans,
            );
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                Some(expr),
                semantic,
                command_scope,
                source,
                spans,
            );
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                Some(left),
                semantic,
                command_scope,
                source,
                spans,
            );
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                Some(right),
                semantic,
                command_scope,
                source,
                spans,
            );
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                Some(condition),
                semantic,
                command_scope,
                source,
                spans,
            );
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                Some(then_expr),
                semantic,
                command_scope,
                source,
                spans,
            );
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                Some(else_expr),
                semantic,
                command_scope,
                source,
                spans,
            );
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            collect_zsh_option_map_suppressed_subscripts_in_lvalue(
                target,
                semantic,
                command_scope,
                source,
                spans,
            );
            collect_zsh_option_map_suppressed_subscripts_in_expr(
                Some(value),
                semantic,
                command_scope,
                source,
                spans,
            );
        }
    }
}

fn collect_zsh_option_map_suppressed_subscripts_in_lvalue(
    target: &ArithmeticLvalue,
    semantic: &SemanticModel,
    command_scope: ScopeId,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { name, index } => {
            if arithmetic_index_uses_zsh_option_map_key_semantics(
                semantic,
                command_scope,
                name,
                index,
                source,
            ) {
                spans.push(index.span);
            } else {
                collect_zsh_option_map_suppressed_subscripts_in_expr(
                    Some(index),
                    semantic,
                    command_scope,
                    source,
                    spans,
                );
            }
        }
    }
}

fn arithmetic_index_uses_zsh_option_map_key_semantics(
    semantic: &SemanticModel,
    command_scope: ScopeId,
    owner_name: &Name,
    index: &ArithmeticExprNode,
    source: &str,
) -> bool {
    if let Some(binding) =
        semantic.visible_assoc_lookup_binding_for_lookup(owner_name, command_scope, index.span)
    {
        if binding
            .attributes
            .contains(shuck_semantic::BindingAttributes::ASSOC)
        {
            return true;
        }
        if !zsh_option_map_binding_origin(owner_name, binding, source)
            || zsh_option_map_binding_has_prior_assoc_lookup_blocker(
                semantic, owner_name, binding, source,
            )
        {
            return false;
        }
    }

    zsh_option_map_binding_permits_implicit_assoc_key(
        semantic,
        semantic.visible_binding_for_lookup(owner_name, command_scope, index.span),
        owner_name,
        source,
    )
        && semantic.shell_profile().dialect == shuck_parser::parser::ShellDialect::Zsh
        && zsh_option_map_subscript_key(owner_name.as_str(), index.span.slice(source))
}

fn zsh_option_map_binding_permits_implicit_assoc_key(
    semantic: &SemanticModel,
    binding: Option<&Binding>,
    owner_name: &Name,
    source: &str,
) -> bool {
    let Some(binding) = binding else {
        return true;
    };
    if binding.attributes.contains(shuck_semantic::BindingAttributes::ASSOC) {
        return true;
    }

    zsh_option_map_binding_origin(owner_name, binding, source)
        && !zsh_option_map_binding_has_prior_assoc_lookup_blocker(
            semantic, owner_name, binding, source,
        )
}

fn zsh_option_map_binding_origin(owner_name: &Name, binding: &Binding, source: &str) -> bool {
    match &binding.origin {
        shuck_semantic::BindingOrigin::Assignment {
            definition_span, ..
        } => zsh_option_map_assignment_target(owner_name, definition_span.slice(source)),
        shuck_semantic::BindingOrigin::ArithmeticAssignment { target_span, .. } => {
            zsh_option_map_assignment_target(owner_name, target_span.slice(source))
        }
        shuck_semantic::BindingOrigin::ParameterDefaultAssignment { .. }
        | shuck_semantic::BindingOrigin::LoopVariable { .. }
        | shuck_semantic::BindingOrigin::Imported { .. }
        | shuck_semantic::BindingOrigin::FunctionDefinition { .. }
        | shuck_semantic::BindingOrigin::BuiltinTarget { .. }
        | shuck_semantic::BindingOrigin::Declaration { .. }
        | shuck_semantic::BindingOrigin::Nameref { .. } => false,
    }
}

fn zsh_option_map_binding_has_prior_assoc_lookup_blocker(
    semantic: &SemanticModel,
    owner_name: &Name,
    binding: &Binding,
    source: &str,
) -> bool {
    semantic.bindings_for(owner_name).iter().copied().any(|id| {
        let candidate = semantic.binding(id);
        candidate.scope == binding.scope
            && candidate.span.start.offset < binding.span.start.offset
            && zsh_option_map_binding_blocks_assoc_lookup(candidate)
            && !zsh_option_map_binding_origin(owner_name, candidate, source)
    })
}

fn zsh_option_map_binding_blocks_assoc_lookup(binding: &Binding) -> bool {
    binding
        .attributes
        .contains(shuck_semantic::BindingAttributes::LOCAL)
        || !matches!(
            binding.kind,
            BindingKind::Assignment
                | BindingKind::AppendAssignment
                | BindingKind::ArrayAssignment
                | BindingKind::ArithmeticAssignment
        )
}

fn zsh_option_map_assignment_target(owner_name: &Name, text: &str) -> bool {
    let Some(rest) = text.strip_prefix(owner_name.as_str()) else {
        return false;
    };
    let Some(subscript) = rest.strip_prefix('[').and_then(|rest| rest.strip_suffix(']')) else {
        return false;
    };

    zsh_option_map_subscript_key(owner_name.as_str(), subscript)
}

fn zsh_option_map_subscript_key(owner_name: &str, text: &str) -> bool {
    if owner_name != "OPTS" {
        return false;
    }

    let text = text.trim();
    let Some((short_option, long_option)) = text.rsplit_once(',') else {
        return false;
    };
    let Some(short_option) = short_option.strip_prefix("opt_-") else {
        return false;
    };
    let Some(long_option) = long_option.strip_prefix("--") else {
        return false;
    };

    !short_option.is_empty()
        && short_option
            .chars()
            .all(|ch| ch == '-' || ch == '_' || ch.is_ascii_alphanumeric())
        && !long_option.is_empty()
        && long_option
            .chars()
            .all(|ch| ch == '-' || ch == '_' || ch.is_ascii_alphanumeric())
}

fn collect_base_prefix_spans_in_command_parts(
    command: &Command,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    match command {
        Command::Simple(command) => {
            for assignment in &command.assignments {
                collect_base_prefix_spans_in_assignment(assignment, source, spans);
            }
            collect_base_prefix_spans_in_word(&command.name, source, spans);
            for word in &command.args {
                collect_base_prefix_spans_in_word(word, source, spans);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                for assignment in &command.assignments {
                    collect_base_prefix_spans_in_assignment(assignment, source, spans);
                }
                if let Some(word) = &command.depth {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Continue(command) => {
                for assignment in &command.assignments {
                    collect_base_prefix_spans_in_assignment(assignment, source, spans);
                }
                if let Some(word) = &command.depth {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Return(command) => {
                for assignment in &command.assignments {
                    collect_base_prefix_spans_in_assignment(assignment, source, spans);
                }
                if let Some(word) = &command.code {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Exit(command) => {
                for assignment in &command.assignments {
                    collect_base_prefix_spans_in_assignment(assignment, source, spans);
                }
                if let Some(word) = &command.code {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
        },
        Command::Decl(command) => {
            for assignment in &command.assignments {
                collect_base_prefix_spans_in_assignment(assignment, source, spans);
            }
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_base_prefix_spans_in_word(word, source, spans);
                    }
                    DeclOperand::Assignment(assignment) => {
                        collect_base_prefix_spans_in_assignment(assignment, source, spans);
                    }
                    DeclOperand::Name(_) => {}
                }
            }
        }
        Command::Compound(command) => match command {
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    for word in words {
                        collect_base_prefix_spans_in_word(word, source, spans);
                    }
                }
            }
            CompoundCommand::Repeat(command) => {
                collect_base_prefix_spans_in_word(&command.count, source, spans);
            }
            CompoundCommand::Foreach(command) => {
                for word in &command.words {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
            CompoundCommand::Arithmetic(command) => {
                if let Some(expression) = &command.expr_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else if let Some(span) = command.expr_span {
                    collect_base_prefix_spans_in_text(span, source, spans);
                }
            }
            CompoundCommand::ArithmeticFor(command) => {
                if let Some(expression) = &command.init_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else if let Some(span) = command.init_span {
                    collect_base_prefix_spans_in_text(span, source, spans);
                }
                if let Some(expression) = &command.condition_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else if let Some(span) = command.condition_span {
                    collect_base_prefix_spans_in_text(span, source, spans);
                }
                if let Some(expression) = &command.step_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else if let Some(span) = command.step_span {
                    collect_base_prefix_spans_in_text(span, source, spans);
                }
            }
            CompoundCommand::Case(command) => {
                collect_base_prefix_spans_in_word(&command.word, source, spans);
                for item in &command.cases {
                    for pattern in &item.patterns {
                        collect_base_prefix_spans_in_pattern(pattern, source, spans);
                    }
                }
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
            }
            CompoundCommand::If(_)
            | CompoundCommand::Conditional(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Subshell(_)
            | CompoundCommand::BraceGroup(_)
            | CompoundCommand::Always(_)
            | CompoundCommand::Coproc(_)
            | CompoundCommand::Time(_) => {}
        },
        Command::Binary(_) | Command::Function(_) | Command::AnonymousFunction(_) => {}
    }
}

fn collect_base_prefix_spans_in_assignment(
    assignment: &Assignment,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    collect_base_prefix_spans_in_var_ref(&assignment.target, source, spans);

    match &assignment.value {
        AssignmentValue::Scalar(word) => collect_base_prefix_spans_in_word(word, source, spans),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => {
                        collect_base_prefix_spans_in_word(word, source, spans);
                    }
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        collect_base_prefix_spans_in_subscript(Some(key), source, spans);
                        collect_base_prefix_spans_in_word(value, source, spans);
                    }
                }
            }
        }
    }
}

fn collect_base_prefix_spans_in_word(word: &Word, source: &str, spans: &mut Vec<(Span, ArithmeticLiteralKind)>) {
    for part in &word.parts {
        collect_base_prefix_spans_in_word_part(part, source, spans);
    }
}

fn collect_base_prefix_spans_in_word_part(
    part: &WordPartNode,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => {
            for part in parts {
                collect_base_prefix_spans_in_word_part(part, source, spans);
            }
        }
        WordPart::ArithmeticExpansion {
            expression: _,
            expression_ast,
            expression_word_ast,
            ..
        } => {
            if let Some(expression) = expression_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else {
                collect_base_prefix_spans_in_arithmetic_word(expression_word_ast, source, spans);
            }
        }
        WordPart::Parameter(parameter) => {
            collect_base_prefix_spans_in_parameter_expansion(parameter, source, spans);
        }
        WordPart::ParameterExpansion { reference, .. }
        | WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::IndirectExpansion { reference, .. }
        | WordPart::Transformation { reference, .. } => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
        }
        WordPart::Substring {
            reference,
            offset_word_ast,
            offset_ast,
            length_word_ast,
            length_ast,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset_word_ast,
            offset_ast,
            length_word_ast,
            length_ast,
            ..
        } => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
            if let Some(expression) = offset_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else {
                collect_base_prefix_spans_in_arithmetic_word(offset_word_ast, source, spans);
            }
            if let Some(expression) = length_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else if let Some(length_word_ast) = length_word_ast {
                collect_base_prefix_spans_in_arithmetic_word(length_word_ast, source, spans);
            }
        }
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::Variable(_)
        | WordPart::PrefixMatch { .. } => {}
        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => {}
    }
}

fn collect_base_prefix_spans_in_parameter_expansion(
    parameter: &shuck_ast::ParameterExpansion,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_base_prefix_spans_in_var_ref(reference, source, spans);
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand,
                operand_word_ast,
                ..
            }
            | BourneParameterExpansion::Operation {
                reference,
                operand,
                operand_word_ast,
                ..
            } => {
                collect_base_prefix_spans_in_var_ref(reference, source, spans);
                collect_base_prefix_spans_in_fragment(
                    operand_word_ast.as_ref(),
                    operand.as_ref(),
                    source,
                    spans,
                );
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_word_ast,
                offset_ast,
                length_word_ast,
                length_ast,
                ..
            } => {
                collect_base_prefix_spans_in_var_ref(reference, source, spans);
                if let Some(expression) = offset_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else {
                    collect_base_prefix_spans_in_arithmetic_word(offset_word_ast, source, spans);
                }
                if let Some(expression) = length_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_base_prefix_spans_in_arithmetic_word(length_word_ast, source, spans);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            collect_base_prefix_spans_in_zsh_target(&syntax.target, source, spans);
            if let Some(operation) = &syntax.operation {
                match operation {
                    shuck_ast::ZshExpansionOperation::Slice { .. }
                    | shuck_ast::ZshExpansionOperation::PatternOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Defaulting { .. }
                    | shuck_ast::ZshExpansionOperation::TrimOperation { .. }
                    | shuck_ast::ZshExpansionOperation::ReplacementOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Unknown { .. } => {}
                }
            }
        }
    }
}

fn collect_base_prefix_spans_in_arithmetic_parameter_expansion(
    parameter: &shuck_ast::ParameterExpansion,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                collect_base_prefix_spans_in_var_ref(reference, source, spans);
            }
            BourneParameterExpansion::Indirect {
                reference,
                operand,
                operand_word_ast,
                ..
            }
            | BourneParameterExpansion::Operation {
                reference,
                operand,
                operand_word_ast,
                ..
            } => {
                collect_base_prefix_spans_in_var_ref(reference, source, spans);
                collect_base_prefix_spans_in_arithmetic_fragment(
                    operand_word_ast.as_ref(),
                    operand.as_ref(),
                    source,
                    spans,
                );
            }
            BourneParameterExpansion::Slice {
                reference,
                offset_word_ast,
                offset_ast,
                length_word_ast,
                length_ast,
                ..
            } => {
                collect_base_prefix_spans_in_var_ref(reference, source, spans);
                if let Some(expression) = offset_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else {
                    collect_base_prefix_spans_in_arithmetic_word(offset_word_ast, source, spans);
                }
                if let Some(expression) = length_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                } else if let Some(length_word_ast) = length_word_ast {
                    collect_base_prefix_spans_in_arithmetic_word(length_word_ast, source, spans);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            collect_base_prefix_spans_in_arithmetic_zsh_target(&syntax.target, source, spans);
            if let Some(operation) = &syntax.operation {
                match operation {
                    shuck_ast::ZshExpansionOperation::Slice { .. }
                    | shuck_ast::ZshExpansionOperation::PatternOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Defaulting { .. }
                    | shuck_ast::ZshExpansionOperation::TrimOperation { .. }
                    | shuck_ast::ZshExpansionOperation::ReplacementOperation { .. }
                    | shuck_ast::ZshExpansionOperation::Unknown { .. } => {}
                }
            }
        }
    }
}

fn collect_base_prefix_spans_in_zsh_target(
    target: &shuck_ast::ZshExpansionTarget,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    match target {
        shuck_ast::ZshExpansionTarget::Reference(reference) => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
        }
        shuck_ast::ZshExpansionTarget::Nested(parameter) => {
            collect_base_prefix_spans_in_parameter_expansion(parameter, source, spans);
        }
        shuck_ast::ZshExpansionTarget::Word(word) => {
            collect_base_prefix_spans_in_word(word, source, spans);
        }
        shuck_ast::ZshExpansionTarget::Empty => {}
    }
}

fn collect_base_prefix_spans_in_arithmetic_zsh_target(
    target: &shuck_ast::ZshExpansionTarget,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    match target {
        shuck_ast::ZshExpansionTarget::Reference(reference) => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
        }
        shuck_ast::ZshExpansionTarget::Nested(parameter) => {
            collect_base_prefix_spans_in_arithmetic_parameter_expansion(parameter, source, spans);
        }
        shuck_ast::ZshExpansionTarget::Word(word) => {
            collect_base_prefix_spans_in_arithmetic_word(word, source, spans);
        }
        shuck_ast::ZshExpansionTarget::Empty => {}
    }
}

fn collect_base_prefix_spans_in_pattern(pattern: &Pattern, source: &str, spans: &mut Vec<(Span, ArithmeticLiteralKind)>) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    collect_base_prefix_spans_in_pattern(pattern, source, spans);
                }
            }
            PatternPart::Word(word) => collect_base_prefix_spans_in_word(word, source, spans),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn collect_base_prefix_spans_in_var_ref(reference: &VarRef, source: &str, spans: &mut Vec<(Span, ArithmeticLiteralKind)>) {
    collect_base_prefix_spans_in_subscript(reference.subscript.as_deref(), source, spans);
}

fn collect_base_prefix_spans_in_subscript(
    subscript: Option<&Subscript>,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    if let Some(expression) = subscript.and_then(|subscript| subscript.arithmetic_ast.as_ref()) {
        collect_base_prefix_spans_in_arithmetic(expression, source, spans);
    }
}

fn collect_base_prefix_spans_in_arithmetic(
    expression: &ArithmeticExprNode,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    match &expression.kind {
        ArithmeticExpr::Number(number) => {
            collect_base_prefix_spans_in_text(number.span(), source, spans);
            collect_leading_zero_integer_spans_in_text(number.span(), source, spans);
        }
        ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => {
            collect_base_prefix_spans_in_arithmetic(index, source, spans);
        }
        ArithmeticExpr::ShellWord(word) => {
            collect_base_prefix_spans_in_arithmetic_word(word, source, spans);
        }
        ArithmeticExpr::Parenthesized { expression } => {
            collect_base_prefix_spans_in_arithmetic(expression, source, spans);
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            collect_base_prefix_spans_in_arithmetic(expr, source, spans);
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            collect_base_prefix_spans_in_arithmetic(left, source, spans);
            collect_base_prefix_spans_in_arithmetic(right, source, spans);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_base_prefix_spans_in_arithmetic(condition, source, spans);
            collect_base_prefix_spans_in_arithmetic(then_expr, source, spans);
            collect_base_prefix_spans_in_arithmetic(else_expr, source, spans);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            collect_base_prefix_spans_in_arithmetic_lvalue(target, source, spans);
            collect_base_prefix_spans_in_arithmetic(value, source, spans);
        }
    }
}

fn collect_base_prefix_spans_in_arithmetic_lvalue(
    target: &ArithmeticLvalue,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => {
            collect_base_prefix_spans_in_arithmetic(index, source, spans);
        }
    }
}

fn collect_base_prefix_spans_in_arithmetic_word(word: &Word, source: &str, spans: &mut Vec<(Span, ArithmeticLiteralKind)>) {
    for part in &word.parts {
        collect_base_prefix_spans_in_arithmetic_word_part(part, source, spans);
    }
}

fn collect_base_prefix_spans_in_arithmetic_word_part(
    part: &WordPartNode,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    match &part.kind {
        WordPart::Literal(_) => {
            collect_base_prefix_spans_in_text(part.span, source, spans);
            collect_leading_zero_integer_spans_in_text(part.span, source, spans);
        }
        WordPart::DoubleQuoted { parts, .. } => {
            for part in parts {
                collect_base_prefix_spans_in_arithmetic_word_part(part, source, spans);
            }
        }
        WordPart::ArithmeticExpansion {
            expression: _,
            expression_ast,
            expression_word_ast,
            ..
        } => {
            if let Some(expression) = expression_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else {
                collect_base_prefix_spans_in_arithmetic_word(expression_word_ast, source, spans);
            }
        }
        WordPart::Parameter(parameter) => {
            collect_base_prefix_spans_in_arithmetic_parameter_expansion(parameter, source, spans);
        }
        WordPart::ParameterExpansion {
            reference,
            operand,
            operand_word_ast,
            ..
        }
        | WordPart::IndirectExpansion {
            reference,
            operand,
            operand_word_ast,
            ..
        } => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
            collect_base_prefix_spans_in_arithmetic_fragment(
                operand_word_ast.as_ref(),
                operand.as_ref(),
                source,
                spans,
            );
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
        }
        WordPart::Substring {
            reference,
            offset_word_ast,
            offset_ast,
            length_word_ast,
            length_ast,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset_word_ast,
            offset_ast,
            length_word_ast,
            length_ast,
            ..
        } => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
            if let Some(expression) = offset_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else {
                collect_base_prefix_spans_in_arithmetic_word(offset_word_ast, source, spans);
            }
            if let Some(expression) = length_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else if let Some(length_word_ast) = length_word_ast {
                collect_base_prefix_spans_in_arithmetic_word(length_word_ast, source, spans);
            }
        }
        WordPart::CommandSubstitution { .. } | WordPart::ProcessSubstitution { .. } => {}
        WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::Variable(_)
        | WordPart::PrefixMatch { .. } => {}
    }
}

fn collect_base_prefix_spans_in_fragment(
    word: Option<&Word>,
    text: Option<&SourceText>,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    let Some(text) = text else {
        return;
    };
    let snippet = text.slice(source);
    if !snippet.contains('#') && !contains_leading_zero_integer(snippet) {
        return;
    }

    debug_assert!(
        word.is_some(),
        "parser-backed fragment text should always carry a word AST"
    );
    let Some(word) = word else {
        collect_leading_zero_integer_spans_in_text(text.span(), source, spans);
        return;
    };
    collect_base_prefix_spans_in_word(word, source, spans);
}

fn collect_base_prefix_spans_in_arithmetic_fragment(
    word: Option<&Word>,
    text: Option<&SourceText>,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    let Some(text) = text else {
        return;
    };
    let snippet = text.slice(source);
    if !snippet.contains('#') && !contains_leading_zero_integer(snippet) {
        return;
    }

    let Some(word) = word else {
        collect_base_prefix_spans_in_text(text.span(), source, spans);
        collect_leading_zero_integer_spans_in_text(text.span(), source, spans);
        return;
    };
    collect_base_prefix_spans_in_arithmetic_word(word, source, spans);
}

fn collect_base_prefix_spans_in_text(
    span: Span,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if !bytes[index].is_ascii_digit() {
            index += 1;
            continue;
        }

        if index > 0 {
            let previous = bytes[index - 1];
            if previous.is_ascii_alphanumeric() || previous == b'_' {
                index += 1;
                continue;
            }
        }

        let mut prefix_end = index;
        while prefix_end < bytes.len() && bytes[prefix_end].is_ascii_digit() {
            prefix_end += 1;
        }

        if prefix_end == bytes.len() || bytes[prefix_end] != b'#' {
            index = prefix_end.max(index + 1);
            continue;
        }

        let mut match_end = prefix_end + 1;
        while match_end < bytes.len() {
            let byte = bytes[match_end];
            if byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'_') {
                match_end += 1;
            } else {
                break;
            }
        }

        let start = span.start.advanced_by(&text[..index]);
        let end = start.advanced_by(&text[index..match_end]);
        spans.push((
            Span::from_positions(start, end),
            ArithmeticLiteralKind::ExplicitBasePrefix,
        ));
        index = match_end;
    }
}

fn collect_leading_zero_integer_spans_in_text(
    span: Span,
    source: &str,
    spans: &mut Vec<(Span, ArithmeticLiteralKind)>,
) {
    let text = span.slice(source);
    let bytes = text.as_bytes();
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] != b'0' {
            index += 1;
            continue;
        }

        if index > 0 {
            let previous = bytes[index - 1];
            if previous.is_ascii_alphanumeric() || previous == b'_' || previous == b'#' {
                index += 1;
                continue;
            }
        }

        if matches!(bytes.get(index + 1), Some(b'x' | b'X')) {
            index += 2;
            continue;
        }

        let mut match_end = index + 1;
        while match_end < bytes.len() && bytes[match_end].is_ascii_digit() {
            match_end += 1;
        }

        if match_end == index + 1 {
            index = match_end;
            continue;
        }

        let start = span.start.advanced_by(&text[..index]);
        let end = start.advanced_by(&text[index..match_end]);
        spans.push((
            Span::from_positions(start, end),
            ArithmeticLiteralKind::LeadingZeroInteger,
        ));
        index = match_end;
    }
}

fn contains_leading_zero_integer(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(2).enumerate().any(|(index, window)| {
        window[0] == b'0'
            && window[1].is_ascii_digit()
            && (index == 0 || {
                let previous = bytes[index - 1];
                !previous.is_ascii_alphanumeric() && previous != b'_' && previous != b'#'
            })
    })
}

fn build_double_paren_grouping_spans(commands: &[CommandFact<'_>], source: &str) -> Vec<Span> {
    commands
        .iter()
        .filter_map(|fact| match fact.command() {
            Command::Compound(CompoundCommand::Subshell(_)) => {
                double_paren_grouping_anchor(fact.span(), source)
            }
            _ => None,
        })
        .collect()
}

fn collect_arithmetic_update_operator_spans_in_command(
    command: &Command,
    semantic: &SemanticModel,
    semantic_artifacts: &LinterSemanticArtifacts<'_>,
    scope: ScopeId,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Simple(command) => {
            for assignment in &command.assignments {
                collect_arithmetic_update_operator_spans_in_assignment(
                    assignment,
                    semantic,
                    semantic_artifacts,
                    scope,
                    source,
                    spans,
                );
            }
            collect_arithmetic_update_operator_spans_in_word(
                &command.name,
                semantic,
                source,
                spans,
            );
            for word in &command.args {
                collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                for assignment in &command.assignments {
                    collect_arithmetic_update_operator_spans_in_assignment(
                        assignment,
                        semantic,
                        semantic_artifacts,
                        scope,
                        source,
                        spans,
                    );
                }
                if let Some(word) = &command.depth {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                }
                for word in &command.extra_args {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                }
            }
            BuiltinCommand::Continue(command) => {
                for assignment in &command.assignments {
                    collect_arithmetic_update_operator_spans_in_assignment(
                        assignment,
                        semantic,
                        semantic_artifacts,
                        scope,
                        source,
                        spans,
                    );
                }
                if let Some(word) = &command.depth {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                }
                for word in &command.extra_args {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                }
            }
            BuiltinCommand::Return(command) => {
                for assignment in &command.assignments {
                    collect_arithmetic_update_operator_spans_in_assignment(
                        assignment,
                        semantic,
                        semantic_artifacts,
                        scope,
                        source,
                        spans,
                    );
                }
                if let Some(word) = &command.code {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                }
                for word in &command.extra_args {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                }
            }
            BuiltinCommand::Exit(command) => {
                for assignment in &command.assignments {
                    collect_arithmetic_update_operator_spans_in_assignment(
                        assignment,
                        semantic,
                        semantic_artifacts,
                        scope,
                        source,
                        spans,
                    );
                }
                if let Some(word) = &command.code {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                }
                for word in &command.extra_args {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                }
            }
        },
        Command::Decl(command) => {
            for assignment in &command.assignments {
                collect_arithmetic_update_operator_spans_in_assignment(
                    assignment,
                    semantic,
                    semantic_artifacts,
                    scope,
                    source,
                    spans,
                );
            }
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_arithmetic_update_operator_spans_in_word(
                            word, semantic, source, spans,
                        );
                    }
                    DeclOperand::Assignment(assignment) => {
                        collect_arithmetic_update_operator_spans_in_assignment(
                            assignment,
                            semantic,
                            semantic_artifacts,
                            scope,
                            source,
                            spans,
                        );
                    }
                    DeclOperand::Name(_) => {}
                }
            }
        }
        Command::Compound(command) => match command {
            CompoundCommand::For(command) => {
                if let Some(words) = &command.words {
                    for word in words {
                        collect_arithmetic_update_operator_spans_in_word(
                            word, semantic, source, spans,
                        );
                    }
                }
            }
            CompoundCommand::Repeat(command) => {
                collect_arithmetic_update_operator_spans_in_word(
                    &command.count,
                    semantic,
                    source,
                    spans,
                );
            }
            CompoundCommand::Foreach(command) => {
                for word in &command.words {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                }
            }
            CompoundCommand::Arithmetic(command) => {
                collect_arithmetic_update_operator_spans(command.expr_ast.as_ref(), source, spans);
            }
            CompoundCommand::ArithmeticFor(command) => {
                collect_arithmetic_update_operator_spans(command.init_ast.as_ref(), source, spans);
                collect_arithmetic_update_operator_spans(
                    command.condition_ast.as_ref(),
                    source,
                    spans,
                );
                collect_arithmetic_update_operator_spans(command.step_ast.as_ref(), source, spans);
            }
            CompoundCommand::Case(command) => {
                collect_arithmetic_update_operator_spans_in_word(
                    &command.word,
                    semantic,
                    source,
                    spans,
                );
                for item in &command.cases {
                    for pattern in &item.patterns {
                        collect_arithmetic_update_operator_spans_in_pattern(
                            pattern, semantic, source, spans,
                        );
                    }
                }
            }
            CompoundCommand::Conditional(command) => {
                collect_arithmetic_update_operator_spans_in_conditional_expr(
                    &command.expression,
                    semantic,
                    source,
                    spans,
                );
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                }
            }
            CompoundCommand::If(_)
            | CompoundCommand::While(_)
            | CompoundCommand::Until(_)
            | CompoundCommand::Subshell(_)
            | CompoundCommand::BraceGroup(_)
            | CompoundCommand::Always(_)
            | CompoundCommand::Coproc(_)
            | CompoundCommand::Time(_) => {}
        },
        Command::Binary(_) | Command::Function(_) | Command::AnonymousFunction(_) => {}
    }
}

fn collect_arithmetic_update_operator_spans_in_assignment(
    assignment: &Assignment,
    semantic: &SemanticModel,
    semantic_artifacts: &LinterSemanticArtifacts<'_>,
    scope: ScopeId,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_in_assignment_target(
        &assignment.target,
        semantic,
        scope,
        source,
        spans,
    );
    let target_is_contextual_assoc =
        var_ref_name_has_visible_assoc_binding_at(&assignment.target, semantic, scope);

    match &assignment.value {
        AssignmentValue::Scalar(word) => {
            collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
        }
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => {
                        collect_arithmetic_update_operator_spans_in_word(
                            word, semantic, source, spans,
                        );
                    }
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        if array.kind != ArrayKind::Associative
                            && !(array.kind == ArrayKind::Contextual && target_is_contextual_assoc)
                        {
                            collect_arithmetic_update_operator_spans_in_subscript(
                                Some(key),
                                source,
                                spans,
                            );
                        }
                        collect_arithmetic_update_operator_spans_in_subscript_words(
                            key,
                            semantic,
                            semantic_artifacts,
                            source,
                            spans,
                        );
                        collect_arithmetic_update_operator_spans_in_word(
                            value, semantic, source, spans,
                        );
                    }
                }
            }
        }
    }
}

fn collect_arithmetic_update_operator_spans_in_assignment_target(
    reference: &VarRef,
    semantic: &SemanticModel,
    scope: ScopeId,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if !var_ref_subscript_has_assoc_semantics_in_scope(reference, semantic, scope) {
        collect_arithmetic_update_operator_spans_in_subscript(
            reference.subscript.as_deref(),
            source,
            spans,
        );
    }
    visit_var_ref_subscript_words_with_source(reference, source, &mut |word| {
        collect_arithmetic_update_operator_spans_from_parts(&word.parts, semantic, source, spans);
    });
}

fn var_ref_subscript_has_assoc_semantics(reference: &VarRef, semantic: &SemanticModel) -> bool {
    let Some(subscript) = reference.subscript.as_deref() else {
        return false;
    };
    if matches!(
        subscript.interpretation,
        shuck_ast::SubscriptInterpretation::Associative
    ) {
        return true;
    }
    if !matches!(
        subscript.interpretation,
        shuck_ast::SubscriptInterpretation::Contextual
    ) {
        return false;
    }

    let scope = semantic.scope_at(subscript.span().start.offset);
    var_ref_name_has_visible_assoc_binding_at(reference, semantic, scope)
}

fn var_ref_subscript_has_assoc_semantics_in_scope(
    reference: &VarRef,
    semantic: &SemanticModel,
    scope: ScopeId,
) -> bool {
    let Some(subscript) = reference.subscript.as_deref() else {
        return false;
    };
    if matches!(
        subscript.interpretation,
        shuck_ast::SubscriptInterpretation::Associative
    ) {
        return true;
    }
    if !matches!(
        subscript.interpretation,
        shuck_ast::SubscriptInterpretation::Contextual
    ) {
        return false;
    }

    var_ref_name_has_visible_assoc_binding_at(reference, semantic, scope)
}

fn var_ref_name_has_visible_assoc_binding_at(
    reference: &VarRef,
    semantic: &SemanticModel,
    scope: ScopeId,
) -> bool {
    semantic
        .assoc_binding_visible_for_lookup(&reference.name, scope, reference.name_span)
}

fn collect_arithmetic_update_operator_spans_in_word(
    word: &Word,
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_from_parts(&word.parts, semantic, source, spans);
}

fn collect_arithmetic_update_operator_spans_in_pattern(
    pattern: &Pattern,
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    collect_arithmetic_update_operator_spans_in_pattern(
                        pattern, semantic, source, spans,
                    );
                }
            }
            PatternPart::Word(word) => {
                collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn collect_arithmetic_update_operator_spans_in_conditional_expr(
    expression: &ConditionalExpr,
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            collect_arithmetic_update_operator_spans_in_conditional_expr(
                &expr.left, semantic, source, spans,
            );
            collect_arithmetic_update_operator_spans_in_conditional_expr(
                &expr.right,
                semantic,
                source,
                spans,
            );
        }
        ConditionalExpr::Unary(expr) => {
            collect_arithmetic_update_operator_spans_in_conditional_expr(
                &expr.expr, semantic, source, spans,
            );
        }
        ConditionalExpr::Parenthesized(expr) => {
            collect_arithmetic_update_operator_spans_in_conditional_expr(
                &expr.expr, semantic, source, spans,
            );
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
        }
        ConditionalExpr::Pattern(pattern) => {
            collect_arithmetic_update_operator_spans_in_pattern(pattern, semantic, source, spans);
        }
        ConditionalExpr::VarRef(reference) => {
            collect_arithmetic_update_operator_spans_in_var_ref(reference, semantic, source, spans);
        }
    }
}

fn collect_arithmetic_update_operator_spans_in_heredoc_body(
    parts: &[shuck_ast::HeredocBodyPartNode],
    semantic: &SemanticModel,
    semantic_artifacts: &LinterSemanticArtifacts<'_>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for part in parts {
        match &part.kind {
            shuck_ast::HeredocBodyPart::ArithmeticExpansion {
                expression_ast,
                expression_word_ast,
                ..
            } => {
                if let Some(expression_ast) = expression_ast.as_ref() {
                    collect_arithmetic_update_operator_spans(Some(expression_ast), source, spans);
                } else {
                    collect_arithmetic_update_operator_spans_in_word(
                        expression_word_ast,
                        semantic,
                        source,
                        spans,
                    );
                }
            }
            shuck_ast::HeredocBodyPart::CommandSubstitution { body, .. } => {
                collect_arithmetic_update_operator_spans_in_nested_command_body(
                    body,
                    semantic_artifacts,
                    semantic,
                    source,
                    spans,
                );
            }
            shuck_ast::HeredocBodyPart::Parameter(parameter) => {
                collect_arithmetic_update_operator_spans_in_parameter_expansion_with_nested_commands(
                    parameter,
                    semantic,
                    semantic_artifacts,
                    source,
                    spans,
                );
            }
            shuck_ast::HeredocBodyPart::Literal(_) | shuck_ast::HeredocBodyPart::Variable(_) => {}
        }
    }
}

fn collect_arithmetic_update_operator_spans_in_nested_command_body(
    body: &StmtSeq,
    semantic_artifacts: &LinterSemanticArtifacts<'_>,
    semantic: &SemanticModel,
    source: &str,
    spans: &mut Vec<Span>,
) {
    semantic_artifacts
        .command_topology()
        .body(body)
        .for_each_command_visit(true, |_, visit| {
            let scope = semantic.scope_at(visit.stmt.span.start.offset);
            collect_arithmetic_update_operator_spans_in_command(
                visit.command,
                semantic,
                semantic_artifacts,
                scope,
                source,
                spans,
            );
            for redirect in visit.redirects {
                if let Some(word) = redirect.word_target() {
                    collect_arithmetic_update_operator_spans_in_word(word, semantic, source, spans);
                } else if let Some(heredoc) = redirect.heredoc()
                    && heredoc.delimiter.expands_body
                {
                    collect_arithmetic_update_operator_spans_in_heredoc_body(
                        &heredoc.body.parts,
                        semantic,
                        semantic_artifacts,
                        source,
                        spans,
                    );
                }
            }
            CommandTopologyTraversal::Descend
        });
}

fn collect_arithmetic_update_operator_spans_in_subscript(
    subscript: Option<&Subscript>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Some(subscript) = subscript else {
        return;
    };
    if matches!(
        subscript.interpretation,
        shuck_ast::SubscriptInterpretation::Associative
    ) {
        return;
    }
    if let Some(expression) = subscript.arithmetic_ast.as_ref() {
        collect_arithmetic_update_operator_spans(Some(expression), source, spans);
    }
}

fn collect_arithmetic_update_operator_spans_in_subscript_words(
    subscript: &Subscript,
    semantic: &SemanticModel,
    semantic_artifacts: &LinterSemanticArtifacts<'_>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    visit_subscript_words(Some(subscript), source, &mut |word| {
        collect_arithmetic_update_operator_spans_from_parts_with_nested_commands(
            &word.parts,
            semantic,
            semantic_artifacts,
            source,
            spans,
        );
    });
}

fn collect_arithmetic_update_operator_spans(
    expression: Option<&ArithmeticExprNode>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Some(expression) = expression else {
        return;
    };

    match &expression.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) | ArithmeticExpr::ShellWord(_) => {}
        ArithmeticExpr::Indexed { index, .. } => {
            collect_arithmetic_update_operator_spans(Some(index), source, spans);
        }
        ArithmeticExpr::Parenthesized { expression } => {
            collect_arithmetic_update_operator_spans(Some(expression), source, spans);
        }
        ArithmeticExpr::Unary { op, expr } => {
            if matches!(
                op,
                ArithmeticUnaryOp::PreIncrement | ArithmeticUnaryOp::PreDecrement
            ) {
                spans.push(find_operator_span(
                    expression.span,
                    source,
                    match op {
                        ArithmeticUnaryOp::PreIncrement => "++",
                        ArithmeticUnaryOp::PreDecrement => "--",
                        ArithmeticUnaryOp::Plus
                        | ArithmeticUnaryOp::Minus
                        | ArithmeticUnaryOp::LogicalNot
                        | ArithmeticUnaryOp::BitwiseNot => unreachable!(),
                    },
                    true,
                ));
            }
            collect_arithmetic_update_operator_spans(Some(expr), source, spans);
        }
        ArithmeticExpr::Postfix { expr, op } => {
            spans.push(find_operator_span(
                expression.span,
                source,
                match op {
                    ArithmeticPostfixOp::Increment => "++",
                    ArithmeticPostfixOp::Decrement => "--",
                },
                false,
            ));
            collect_arithmetic_update_operator_spans(Some(expr), source, spans);
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            collect_arithmetic_update_operator_spans(Some(left), source, spans);
            collect_arithmetic_update_operator_spans(Some(right), source, spans);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_arithmetic_update_operator_spans(Some(condition), source, spans);
            collect_arithmetic_update_operator_spans(Some(then_expr), source, spans);
            collect_arithmetic_update_operator_spans(Some(else_expr), source, spans);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            collect_arithmetic_lvalue_update_operator_spans(target, source, spans);
            collect_arithmetic_update_operator_spans(Some(value), source, spans);
        }
    }
}

fn collect_arithmetic_lvalue_update_operator_spans(
    target: &ArithmeticLvalue,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => {
            collect_arithmetic_update_operator_spans(Some(index), source, spans);
        }
    }
}

fn find_operator_span(expression_span: Span, source: &str, operator: &str, first: bool) -> Span {
    let expression = expression_span.slice(source);
    let offset = if first {
        let Some(offset) = expression.find(operator) else {
            unreachable!("expected prefix update operator in arithmetic expression");
        };
        offset
    } else {
        let Some(offset) = expression.rfind(operator) else {
            unreachable!("expected postfix update operator in arithmetic expression");
        };
        offset
    };
    let start = expression_span.start.advanced_by(&expression[..offset]);
    Span::from_positions(start, start.advanced_by(operator))
}

fn double_paren_grouping_anchor(span: Span, source: &str) -> Option<Span> {
    let text = span.slice(source);
    let anchor_start = if let Some(stripped) = text.strip_prefix("((") {
        let body_start =
            (text.len() - stripped.len()) + stripped.find(|char: char| !char.is_whitespace())?;
        let body = &text[body_start..];
        let has_grouping_operator =
            body.contains("||") || body.contains("&&") || body.contains('|') || body.contains(';');
        if !has_grouping_operator {
            return None;
        }
        span.start
    } else if text.starts_with('(')
        && span.start.offset > 0
        && source.as_bytes().get(span.start.offset - 1) == Some(&b'(')
    {
        let stripped = text.strip_prefix('(')?;
        let body_start =
            (text.len() - stripped.len()) + stripped.find(|char: char| !char.is_whitespace())?;
        let body = &text[body_start..];
        let has_grouping_operator =
            body.contains("||") || body.contains("&&") || body.contains('|') || body.contains(';');
        if !has_grouping_operator {
            return None;
        }
        Position {
            line: span.start.line,
            column: span.start.column - 1,
            offset: span.start.offset - 1,
        }
    } else {
        return None;
    };

    Some(Span::at(anchor_start))
}
