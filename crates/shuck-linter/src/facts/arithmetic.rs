fn build_base_prefix_arithmetic_spans(body: &StmtSeq, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();

    for visit in query::iter_commands(
        body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
    ) {
        collect_base_prefix_spans_in_command(visit.command, source, &mut spans);
        for redirect in visit.redirects {
            if let Some(word) = redirect.word_target() {
                collect_base_prefix_spans_in_word(word, source, &mut spans);
            }
        }
    }

    spans
}

fn collect_base_prefix_spans_in_command(command: &Command, source: &str, spans: &mut Vec<Span>) {
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
                    collect_base_prefix_spans_in_stmt_seq(&item.body, source, spans);
                }
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    collect_base_prefix_spans_in_word(word, source, spans);
                }
                collect_base_prefix_spans_in_stmt_seq(&command.body, source, spans);
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
    spans: &mut Vec<Span>,
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

fn collect_base_prefix_spans_in_word(word: &Word, source: &str, spans: &mut Vec<Span>) {
    for part in &word.parts {
        collect_base_prefix_spans_in_word_part(part, source, spans);
    }
}

fn collect_base_prefix_spans_in_word_part(
    part: &WordPartNode,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match &part.kind {
        WordPart::DoubleQuoted { parts, .. } => {
            for part in parts {
                collect_base_prefix_spans_in_word_part(part, source, spans);
            }
        }
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            ..
        } => {
            if let Some(expression) = expression_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else {
                collect_base_prefix_spans_in_text(expression.span(), source, spans);
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
            offset,
            offset_ast,
            length,
            length_ast,
            ..
        }
        | WordPart::ArraySlice {
            reference,
            offset,
            offset_ast,
            length,
            length_ast,
            ..
        } => {
            collect_base_prefix_spans_in_var_ref(reference, source, spans);
            if let Some(expression) = offset_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else {
                collect_base_prefix_spans_in_text(offset.span(), source, spans);
            }
            if let Some(expression) = length_ast {
                collect_base_prefix_spans_in_arithmetic(expression, source, spans);
            } else if let Some(length) = length {
                collect_base_prefix_spans_in_text(length.span(), source, spans);
            }
        }
        WordPart::Literal(_)
        | WordPart::ZshQualifiedGlob(_)
        | WordPart::SingleQuoted { .. }
        | WordPart::Variable(_)
        | WordPart::PrefixMatch { .. } => {}
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            collect_base_prefix_spans_in_stmt_seq(body, source, spans);
        }
    }
}

fn collect_base_prefix_spans_in_parameter_expansion(
    parameter: &shuck_ast::ParameterExpansion,
    source: &str,
    spans: &mut Vec<Span>,
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
                offset_ast,
                length_ast,
                ..
            } => {
                collect_base_prefix_spans_in_var_ref(reference, source, spans);
                if let Some(expression) = offset_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
                }
                if let Some(expression) = length_ast {
                    collect_base_prefix_spans_in_arithmetic(expression, source, spans);
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

fn collect_base_prefix_spans_in_zsh_target(
    target: &shuck_ast::ZshExpansionTarget,
    source: &str,
    spans: &mut Vec<Span>,
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

fn collect_base_prefix_spans_in_stmt_seq(body: &StmtSeq, source: &str, spans: &mut Vec<Span>) {
    for stmt in &body.stmts {
        collect_base_prefix_spans_in_command(&stmt.command, source, spans);
    }
}

fn collect_base_prefix_spans_in_pattern(pattern: &Pattern, source: &str, spans: &mut Vec<Span>) {
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

fn collect_base_prefix_spans_in_var_ref(reference: &VarRef, source: &str, spans: &mut Vec<Span>) {
    collect_base_prefix_spans_in_subscript(reference.subscript.as_ref(), source, spans);
}

fn collect_base_prefix_spans_in_subscript(
    subscript: Option<&Subscript>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if let Some(expression) = subscript.and_then(|subscript| subscript.arithmetic_ast.as_ref()) {
        collect_base_prefix_spans_in_arithmetic(expression, source, spans);
    }
}

fn collect_base_prefix_spans_in_arithmetic(
    expression: &ArithmeticExprNode,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_base_prefix_spans_in_text(expression.span, source, spans);
}

fn collect_base_prefix_spans_in_fragment(
    word: Option<&Word>,
    text: Option<&SourceText>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    let Some(text) = text else {
        return;
    };
    let snippet = text.slice(source);
    if !snippet.contains('#') {
        return;
    }

    debug_assert!(
        word.is_some(),
        "parser-backed fragment text should always carry a word AST"
    );
    let Some(word) = word else {
        return;
    };
    collect_base_prefix_spans_in_word(word, source, spans);
}

fn collect_base_prefix_spans_in_text(span: Span, source: &str, spans: &mut Vec<Span>) {
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
        spans.push(Span::from_positions(start, end));
        index = match_end;
    }
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

fn build_arithmetic_update_operator_spans(body: &StmtSeq, source: &str) -> Vec<Span> {
    let mut spans = Vec::new();

    for visit in query::iter_commands(
        body,
        CommandWalkOptions {
            descend_nested_word_commands: true,
        },
    ) {
        collect_arithmetic_update_operator_spans_in_command(visit.command, source, &mut spans);
        for redirect in visit.redirects {
            if let Some(word) = redirect.word_target() {
                collect_arithmetic_update_operator_spans_in_word(word, source, &mut spans);
            } else if let Some(heredoc) = redirect.heredoc()
                && heredoc.delimiter.expands_body
            {
                collect_arithmetic_update_operator_spans_in_heredoc_body(
                    &heredoc.body.parts,
                    source,
                    &mut spans,
                );
            }
        }
    }

    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
    spans
}

fn collect_arithmetic_update_operator_spans_in_command(
    command: &Command,
    source: &str,
    spans: &mut Vec<Span>,
) {
    match command {
        Command::Simple(command) => {
            for assignment in &command.assignments {
                collect_arithmetic_update_operator_spans_in_assignment(assignment, source, spans);
            }
            collect_arithmetic_update_operator_spans_in_word(&command.name, source, spans);
            for word in &command.args {
                collect_arithmetic_update_operator_spans_in_word(word, source, spans);
            }
        }
        Command::Builtin(command) => match command {
            BuiltinCommand::Break(command) => {
                for assignment in &command.assignments {
                    collect_arithmetic_update_operator_spans_in_assignment(
                        assignment, source, spans,
                    );
                }
                if let Some(word) = &command.depth {
                    collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Continue(command) => {
                for assignment in &command.assignments {
                    collect_arithmetic_update_operator_spans_in_assignment(
                        assignment, source, spans,
                    );
                }
                if let Some(word) = &command.depth {
                    collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Return(command) => {
                for assignment in &command.assignments {
                    collect_arithmetic_update_operator_spans_in_assignment(
                        assignment, source, spans,
                    );
                }
                if let Some(word) = &command.code {
                    collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                }
            }
            BuiltinCommand::Exit(command) => {
                for assignment in &command.assignments {
                    collect_arithmetic_update_operator_spans_in_assignment(
                        assignment, source, spans,
                    );
                }
                if let Some(word) = &command.code {
                    collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                }
                for word in &command.extra_args {
                    collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                }
            }
        },
        Command::Decl(command) => {
            for assignment in &command.assignments {
                collect_arithmetic_update_operator_spans_in_assignment(assignment, source, spans);
            }
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                    }
                    DeclOperand::Assignment(assignment) => {
                        collect_arithmetic_update_operator_spans_in_assignment(
                            assignment, source, spans,
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
                        collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                    }
                }
            }
            CompoundCommand::Repeat(command) => {
                collect_arithmetic_update_operator_spans_in_word(&command.count, source, spans);
            }
            CompoundCommand::Foreach(command) => {
                for word in &command.words {
                    collect_arithmetic_update_operator_spans_in_word(word, source, spans);
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
                collect_arithmetic_update_operator_spans_in_word(&command.word, source, spans);
                for item in &command.cases {
                    for pattern in &item.patterns {
                        collect_arithmetic_update_operator_spans_in_pattern(pattern, source, spans);
                    }
                }
            }
            CompoundCommand::Select(command) => {
                for word in &command.words {
                    collect_arithmetic_update_operator_spans_in_word(word, source, spans);
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

fn collect_arithmetic_update_operator_spans_in_assignment(
    assignment: &Assignment,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_in_var_ref(&assignment.target, source, spans);

    match &assignment.value {
        AssignmentValue::Scalar(word) => {
            collect_arithmetic_update_operator_spans_in_word(word, source, spans);
        }
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => {
                        collect_arithmetic_update_operator_spans_in_word(word, source, spans);
                    }
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        if array.kind != ArrayKind::Associative {
                            collect_arithmetic_update_operator_spans_in_subscript(
                                Some(key),
                                source,
                                spans,
                            );
                        }
                        collect_arithmetic_update_operator_spans_in_word(value, source, spans);
                    }
                }
            }
        }
    }
}

fn collect_arithmetic_update_operator_spans_in_word(
    word: &Word,
    source: &str,
    spans: &mut Vec<Span>,
) {
    collect_arithmetic_update_operator_spans_from_parts(&word.parts, source, spans);
}

fn collect_arithmetic_update_operator_spans_in_pattern(
    pattern: &Pattern,
    source: &str,
    spans: &mut Vec<Span>,
) {
    for (part, _) in pattern.parts_with_spans() {
        match part {
            PatternPart::Group { patterns, .. } => {
                for pattern in patterns {
                    collect_arithmetic_update_operator_spans_in_pattern(pattern, source, spans);
                }
            }
            PatternPart::Word(word) => {
                collect_arithmetic_update_operator_spans_in_word(word, source, spans);
            }
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn collect_arithmetic_update_operator_spans_in_heredoc_body(
    parts: &[shuck_ast::HeredocBodyPartNode],
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
                        source,
                        spans,
                    );
                }
            }
            shuck_ast::HeredocBodyPart::CommandSubstitution { body, .. } => {
                for stmt in &body.stmts {
                    collect_arithmetic_update_operator_spans_in_command(
                        &stmt.command,
                        source,
                        spans,
                    );
                    for redirect in &stmt.redirects {
                        if let Some(word) = redirect.word_target() {
                            collect_arithmetic_update_operator_spans_in_word(
                                word,
                                source,
                                spans,
                            );
                        } else if let Some(heredoc) = redirect.heredoc()
                            && heredoc.delimiter.expands_body
                        {
                            collect_arithmetic_update_operator_spans_in_heredoc_body(
                                &heredoc.body.parts,
                                source,
                                spans,
                            );
                        }
                    }
                }
            }
            shuck_ast::HeredocBodyPart::Parameter(parameter) => {
                collect_arithmetic_update_operator_spans_in_parameter_expansion(
                    parameter, source, spans,
                );
            }
            shuck_ast::HeredocBodyPart::Literal(_) | shuck_ast::HeredocBodyPart::Variable(_) => {}
        }
    }
}

fn collect_arithmetic_update_operator_spans_in_subscript(
    subscript: Option<&Subscript>,
    source: &str,
    spans: &mut Vec<Span>,
) {
    if let Some(expression) = subscript.and_then(|subscript| subscript.arithmetic_ast.as_ref()) {
        collect_arithmetic_update_operator_spans(Some(expression), source, spans);
    }
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
