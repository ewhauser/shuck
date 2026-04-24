use std::process;

use serde::Serialize;
use shuck_ast::*;
use shuck_benchmark::{benchmark_cases, parse_fixture};
use shuck_parser::parser::ParseStatus;

#[derive(Debug, Default, Serialize)]
struct AstShape {
    stmt_sequences: u64,
    statements: u64,
    commands: u64,
    words: u64,
    word_parts: u64,
    patterns: u64,
    pattern_parts: u64,
    arithmetic_exprs: u64,
    conditional_exprs: u64,
    redirects: u64,
    heredocs: u64,
    assignments: u64,
    array_exprs: u64,
    array_elems: u64,
    var_refs: u64,
    parameter_expansions: u64,
    source_text_fields: u64,
    non_source_text_fields: u64,
    literal_text_fields: u64,
    owned_literal_text_fields: u64,
    string_fields: u64,
    non_empty_string_fields: u64,
    box_edges: u64,
    vec_fields: u64,
    non_empty_vec_fields: u64,
    vec_elements: u64,
    max_stmt_seq_depth: u64,
    max_word_depth: u64,
    max_arithmetic_depth: u64,
    max_conditional_depth: u64,
}

impl AstShape {
    fn estimated_replaceable_heap_allocations(&self) -> u64 {
        self.box_edges
            + self.non_empty_vec_fields
            + self.non_source_text_fields
            + self.owned_literal_text_fields
            + self.non_empty_string_fields
    }

    fn record_vec<T>(&mut self, values: &[T]) {
        self.vec_fields += 1;
        self.vec_elements += values.len() as u64;
        if !values.is_empty() {
            self.non_empty_vec_fields += 1;
        }
    }

    fn record_option_vec<T>(&mut self, values: &Option<Vec<T>>) {
        self.vec_fields += 1;
        if let Some(values) = values {
            self.vec_elements += values.len() as u64;
            if !values.is_empty() {
                self.non_empty_vec_fields += 1;
            }
        }
    }

    fn record_box(&mut self) {
        self.box_edges += 1;
    }

    fn record_source_text(&mut self, text: &SourceText) {
        self.source_text_fields += 1;
        if !text.is_source_backed() {
            self.non_source_text_fields += 1;
        }
    }

    fn record_literal_text(&mut self, text: &LiteralText) {
        self.literal_text_fields += 1;
        if !text.is_source_backed() {
            self.owned_literal_text_fields += 1;
        }
    }

    fn record_string(&mut self, value: &str) {
        self.string_fields += 1;
        if !value.is_empty() {
            self.non_empty_string_fields += 1;
        }
    }
}

#[derive(Debug, Serialize)]
struct CaseReport {
    case: String,
    files: usize,
    recovered_files: usize,
    command_count: usize,
    estimated_replaceable_heap_allocations: u64,
    shape: AstShape,
}

fn visit_file(file: &File, shape: &mut AstShape) {
    visit_stmt_seq(&file.body, shape, 1);
}

fn visit_stmt_seq(seq: &StmtSeq, shape: &mut AstShape, depth: u64) {
    shape.stmt_sequences += 1;
    shape.max_stmt_seq_depth = shape.max_stmt_seq_depth.max(depth);
    shape.record_vec(&seq.leading_comments);
    shape.record_vec(&seq.stmts);
    shape.record_vec(&seq.trailing_comments);
    for stmt in &seq.stmts {
        visit_stmt(stmt, shape, depth);
    }
}

fn visit_stmt(stmt: &Stmt, shape: &mut AstShape, depth: u64) {
    shape.statements += 1;
    shape.record_vec(&stmt.leading_comments);
    visit_command(&stmt.command, shape, depth);
    shape.record_vec(&stmt.redirects);
    for redirect in &stmt.redirects {
        visit_redirect(redirect, shape);
    }
}

fn visit_command(command: &Command, shape: &mut AstShape, depth: u64) {
    shape.commands += 1;
    match command {
        Command::Simple(command) => {
            visit_word(&command.name, shape, 1);
            shape.record_vec(&command.args);
            for arg in &command.args {
                visit_word(arg, shape, 1);
            }
            shape.record_vec(&command.assignments);
            for assignment in &command.assignments {
                visit_assignment(assignment, shape);
            }
        }
        Command::Builtin(command) => visit_builtin_command(command, shape),
        Command::Decl(command) => {
            shape.record_vec(&command.operands);
            for operand in &command.operands {
                visit_decl_operand(operand, shape);
            }
            shape.record_vec(&command.assignments);
            for assignment in &command.assignments {
                visit_assignment(assignment, shape);
            }
        }
        Command::Binary(command) => {
            shape.record_box();
            visit_stmt(&command.left, shape, depth + 1);
            shape.record_box();
            visit_stmt(&command.right, shape, depth + 1);
        }
        Command::Compound(command) => visit_compound_command(command, shape, depth),
        Command::Function(command) => {
            visit_function_header(&command.header, shape);
            shape.record_box();
            visit_stmt(&command.body, shape, depth + 1);
        }
        Command::AnonymousFunction(command) => {
            shape.record_box();
            visit_stmt(&command.body, shape, depth + 1);
            shape.record_vec(&command.args);
            for arg in &command.args {
                visit_word(arg, shape, 1);
            }
        }
    }
}

fn visit_builtin_command(command: &BuiltinCommand, shape: &mut AstShape) {
    match command {
        BuiltinCommand::Break(command) => {
            if let Some(depth) = &command.depth {
                visit_word(depth, shape, 1);
            }
            shape.record_vec(&command.extra_args);
            for arg in &command.extra_args {
                visit_word(arg, shape, 1);
            }
            shape.record_vec(&command.assignments);
            for assignment in &command.assignments {
                visit_assignment(assignment, shape);
            }
        }
        BuiltinCommand::Continue(command) => {
            if let Some(depth) = &command.depth {
                visit_word(depth, shape, 1);
            }
            shape.record_vec(&command.extra_args);
            for arg in &command.extra_args {
                visit_word(arg, shape, 1);
            }
            shape.record_vec(&command.assignments);
            for assignment in &command.assignments {
                visit_assignment(assignment, shape);
            }
        }
        BuiltinCommand::Return(command) => {
            if let Some(code) = &command.code {
                visit_word(code, shape, 1);
            }
            shape.record_vec(&command.extra_args);
            for arg in &command.extra_args {
                visit_word(arg, shape, 1);
            }
            shape.record_vec(&command.assignments);
            for assignment in &command.assignments {
                visit_assignment(assignment, shape);
            }
        }
        BuiltinCommand::Exit(command) => {
            if let Some(code) = &command.code {
                visit_word(code, shape, 1);
            }
            shape.record_vec(&command.extra_args);
            for arg in &command.extra_args {
                visit_word(arg, shape, 1);
            }
            shape.record_vec(&command.assignments);
            for assignment in &command.assignments {
                visit_assignment(assignment, shape);
            }
        }
    }
}

fn visit_decl_operand(operand: &DeclOperand, shape: &mut AstShape) {
    match operand {
        DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => visit_word(word, shape, 1),
        DeclOperand::Name(reference) => visit_var_ref(reference, shape),
        DeclOperand::Assignment(assignment) => visit_assignment(assignment, shape),
    }
}

fn visit_compound_command(command: &CompoundCommand, shape: &mut AstShape, depth: u64) {
    match command {
        CompoundCommand::If(command) => {
            visit_stmt_seq(&command.condition, shape, depth + 1);
            visit_stmt_seq(&command.then_branch, shape, depth + 1);
            shape.record_vec(&command.elif_branches);
            for (condition, branch) in &command.elif_branches {
                visit_stmt_seq(condition, shape, depth + 1);
                visit_stmt_seq(branch, shape, depth + 1);
            }
            if let Some(else_branch) = &command.else_branch {
                visit_stmt_seq(else_branch, shape, depth + 1);
            }
        }
        CompoundCommand::For(command) => {
            shape.record_vec(&command.targets);
            for target in &command.targets {
                visit_word(&target.word, shape, 1);
            }
            shape.record_option_vec(&command.words);
            if let Some(words) = &command.words {
                for word in words {
                    visit_word(word, shape, 1);
                }
            }
            visit_stmt_seq(&command.body, shape, depth + 1);
        }
        CompoundCommand::Repeat(command) => {
            visit_word(&command.count, shape, 1);
            visit_stmt_seq(&command.body, shape, depth + 1);
        }
        CompoundCommand::Foreach(command) => {
            shape.record_vec(&command.words);
            for word in &command.words {
                visit_word(word, shape, 1);
            }
            visit_stmt_seq(&command.body, shape, depth + 1);
        }
        CompoundCommand::ArithmeticFor(command) => {
            shape.record_box();
            if let Some(expr) = &command.init_ast {
                visit_arithmetic_expr(expr, shape, 1);
            }
            if let Some(expr) = &command.condition_ast {
                visit_arithmetic_expr(expr, shape, 1);
            }
            if let Some(expr) = &command.step_ast {
                visit_arithmetic_expr(expr, shape, 1);
            }
            visit_stmt_seq(&command.body, shape, depth + 1);
        }
        CompoundCommand::While(command) => {
            visit_stmt_seq(&command.condition, shape, depth + 1);
            visit_stmt_seq(&command.body, shape, depth + 1);
        }
        CompoundCommand::Until(command) => {
            visit_stmt_seq(&command.condition, shape, depth + 1);
            visit_stmt_seq(&command.body, shape, depth + 1);
        }
        CompoundCommand::Case(command) => {
            visit_word(&command.word, shape, 1);
            shape.record_vec(&command.cases);
            for case in &command.cases {
                shape.record_vec(&case.patterns);
                for pattern in &case.patterns {
                    visit_pattern(pattern, shape);
                }
                visit_stmt_seq(&case.body, shape, depth + 1);
            }
        }
        CompoundCommand::Select(command) => {
            shape.record_vec(&command.words);
            for word in &command.words {
                visit_word(word, shape, 1);
            }
            visit_stmt_seq(&command.body, shape, depth + 1);
        }
        CompoundCommand::Subshell(seq) | CompoundCommand::BraceGroup(seq) => {
            visit_stmt_seq(seq, shape, depth + 1);
        }
        CompoundCommand::Arithmetic(command) => {
            if let Some(expr) = &command.expr_ast {
                visit_arithmetic_expr(expr, shape, 1);
            }
        }
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                shape.record_box();
                visit_stmt(command, shape, depth + 1);
            }
        }
        CompoundCommand::Conditional(command) => {
            visit_conditional_expr(&command.expression, shape, 1);
        }
        CompoundCommand::Coproc(command) => {
            shape.record_box();
            visit_stmt(&command.body, shape, depth + 1);
        }
        CompoundCommand::Always(command) => {
            visit_stmt_seq(&command.body, shape, depth + 1);
            visit_stmt_seq(&command.always_body, shape, depth + 1);
        }
    }
}

fn visit_function_header(header: &FunctionHeader, shape: &mut AstShape) {
    shape.record_vec(&header.entries);
    for entry in &header.entries {
        visit_word(&entry.word, shape, 1);
    }
}

fn visit_redirect(redirect: &Redirect, shape: &mut AstShape) {
    shape.redirects += 1;
    match &redirect.target {
        RedirectTarget::Word(word) => visit_word(word, shape, 1),
        RedirectTarget::Heredoc(heredoc) => visit_heredoc(heredoc, shape),
    }
}

fn visit_heredoc(heredoc: &Heredoc, shape: &mut AstShape) {
    shape.heredocs += 1;
    visit_word(&heredoc.delimiter.raw, shape, 1);
    shape.record_string(&heredoc.delimiter.cooked);
    visit_heredoc_body(&heredoc.body, shape);
}

fn visit_heredoc_body(body: &HeredocBody, shape: &mut AstShape) {
    shape.record_vec(&body.parts);
    for part in &body.parts {
        match &part.kind {
            HeredocBodyPart::Literal(text) => shape.record_literal_text(text),
            HeredocBodyPart::Variable(_) => {}
            HeredocBodyPart::CommandSubstitution { body, .. } => visit_stmt_seq(body, shape, 1),
            HeredocBodyPart::ArithmeticExpansion {
                expression,
                expression_ast,
                expression_word_ast,
                ..
            } => {
                shape.record_source_text(expression);
                if let Some(expr) = expression_ast {
                    visit_arithmetic_expr(expr, shape, 1);
                }
                visit_word(expression_word_ast, shape, 1);
            }
            HeredocBodyPart::Parameter(parameter) => {
                shape.record_box();
                visit_parameter_expansion(parameter, shape);
            }
        }
    }
}

fn visit_assignment(assignment: &Assignment, shape: &mut AstShape) {
    shape.assignments += 1;
    visit_var_ref(&assignment.target, shape);
    match &assignment.value {
        AssignmentValue::Scalar(word) => visit_word(word, shape, 1),
        AssignmentValue::Compound(array) => visit_array_expr(array, shape),
    }
}

fn visit_array_expr(array: &ArrayExpr, shape: &mut AstShape) {
    shape.array_exprs += 1;
    shape.record_vec(&array.elements);
    for element in &array.elements {
        shape.array_elems += 1;
        match element {
            ArrayElem::Sequential(word) => visit_word(&word.word, shape, 1),
            ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                visit_subscript(key, shape);
                visit_word(&value.word, shape, 1);
            }
        }
    }
}

fn visit_var_ref(reference: &VarRef, shape: &mut AstShape) {
    shape.var_refs += 1;
    if let Some(subscript) = &reference.subscript {
        visit_subscript(subscript, shape);
    }
}

fn visit_subscript(subscript: &Subscript, shape: &mut AstShape) {
    shape.record_source_text(&subscript.text);
    if let Some(raw) = &subscript.raw {
        shape.record_source_text(raw);
    }
    if let Some(word) = &subscript.word_ast {
        visit_word(word, shape, 1);
    }
    if let Some(expr) = &subscript.arithmetic_ast {
        visit_arithmetic_expr(expr, shape, 1);
    }
}

fn visit_word(word: &Word, shape: &mut AstShape, depth: u64) {
    shape.words += 1;
    shape.max_word_depth = shape.max_word_depth.max(depth);
    shape.record_vec(&word.parts);
    shape.record_vec(&word.brace_syntax);
    for part in &word.parts {
        shape.word_parts += 1;
        visit_word_part(&part.kind, shape, depth);
    }
}

fn visit_word_part(part: &WordPart, shape: &mut AstShape, depth: u64) {
    match part {
        WordPart::Literal(text) => shape.record_literal_text(text),
        WordPart::ZshQualifiedGlob(glob) => visit_zsh_qualified_glob(glob, shape),
        WordPart::SingleQuoted { value, .. } => shape.record_source_text(value),
        WordPart::DoubleQuoted { parts, .. } => {
            shape.record_vec(parts);
            for part in parts {
                shape.word_parts += 1;
                visit_word_part(&part.kind, shape, depth + 1);
            }
        }
        WordPart::Variable(_) => {}
        WordPart::CommandSubstitution { body, .. } | WordPart::ProcessSubstitution { body, .. } => {
            visit_stmt_seq(body, shape, 1)
        }
        WordPart::ArithmeticExpansion {
            expression,
            expression_ast,
            expression_word_ast,
            ..
        } => {
            shape.record_source_text(expression);
            if let Some(expr) = expression_ast {
                visit_arithmetic_expr(expr, shape, 1);
            }
            visit_word(expression_word_ast, shape, depth + 1);
        }
        WordPart::Parameter(parameter) => visit_parameter_expansion(parameter, shape),
        WordPart::ParameterExpansion {
            reference,
            operator,
            operand,
            operand_word_ast,
            ..
        } => {
            visit_var_ref(reference, shape);
            visit_parameter_op(operator, shape);
            if let Some(operand) = operand {
                shape.record_source_text(operand);
            }
            if let Some(word) = operand_word_ast {
                visit_word(word, shape, depth + 1);
            }
        }
        WordPart::Length(reference)
        | WordPart::ArrayAccess(reference)
        | WordPart::ArrayLength(reference)
        | WordPart::ArrayIndices(reference)
        | WordPart::Transformation { reference, .. } => visit_var_ref(reference, shape),
        WordPart::Substring {
            reference,
            offset,
            offset_ast,
            offset_word_ast,
            length,
            length_ast,
            length_word_ast,
        }
        | WordPart::ArraySlice {
            reference,
            offset,
            offset_ast,
            offset_word_ast,
            length,
            length_ast,
            length_word_ast,
        } => {
            visit_var_ref(reference, shape);
            shape.record_source_text(offset);
            if let Some(expr) = offset_ast {
                visit_arithmetic_expr(expr, shape, 1);
            }
            visit_word(offset_word_ast, shape, depth + 1);
            if let Some(length) = length {
                shape.record_source_text(length);
            }
            if let Some(expr) = length_ast {
                visit_arithmetic_expr(expr, shape, 1);
            }
            if let Some(word) = length_word_ast {
                visit_word(word, shape, depth + 1);
            }
        }
        WordPart::IndirectExpansion {
            reference,
            operator,
            operand,
            operand_word_ast,
            ..
        } => {
            visit_var_ref(reference, shape);
            if let Some(operator) = operator {
                visit_parameter_op(operator, shape);
            }
            if let Some(operand) = operand {
                shape.record_source_text(operand);
            }
            if let Some(word) = operand_word_ast {
                visit_word(word, shape, depth + 1);
            }
        }
        WordPart::PrefixMatch { .. } => {}
    }
}

fn visit_parameter_expansion(parameter: &ParameterExpansion, shape: &mut AstShape) {
    shape.parameter_expansions += 1;
    shape.record_source_text(&parameter.raw_body);
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(expansion) => visit_bourne_expansion(expansion, shape),
        ParameterExpansionSyntax::Zsh(expansion) => visit_zsh_parameter_expansion(expansion, shape),
    }
}

fn visit_bourne_expansion(expansion: &BourneParameterExpansion, shape: &mut AstShape) {
    match expansion {
        BourneParameterExpansion::Access { reference }
        | BourneParameterExpansion::Length { reference }
        | BourneParameterExpansion::Indices { reference }
        | BourneParameterExpansion::Transformation { reference, .. } => {
            visit_var_ref(reference, shape);
        }
        BourneParameterExpansion::Indirect {
            reference,
            operator,
            operand,
            operand_word_ast,
            ..
        } => {
            visit_var_ref(reference, shape);
            if let Some(operator) = operator {
                visit_parameter_op(operator, shape);
            }
            if let Some(operand) = operand {
                shape.record_source_text(operand);
            }
            if let Some(word) = operand_word_ast {
                visit_word(word, shape, 1);
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
        } => {
            visit_var_ref(reference, shape);
            shape.record_source_text(offset);
            if let Some(expr) = offset_ast {
                visit_arithmetic_expr(expr, shape, 1);
            }
            visit_word(offset_word_ast, shape, 1);
            if let Some(length) = length {
                shape.record_source_text(length);
            }
            if let Some(expr) = length_ast {
                visit_arithmetic_expr(expr, shape, 1);
            }
            if let Some(word) = length_word_ast {
                visit_word(word, shape, 1);
            }
        }
        BourneParameterExpansion::Operation {
            reference,
            operator,
            operand,
            operand_word_ast,
            ..
        } => {
            visit_var_ref(reference, shape);
            visit_parameter_op(operator, shape);
            if let Some(operand) = operand {
                shape.record_source_text(operand);
            }
            if let Some(word) = operand_word_ast {
                visit_word(word, shape, 1);
            }
        }
    }
}

fn visit_zsh_parameter_expansion(expansion: &ZshParameterExpansion, shape: &mut AstShape) {
    visit_zsh_target(&expansion.target, shape);
    shape.record_vec(&expansion.modifiers);
    for modifier in &expansion.modifiers {
        if let Some(argument) = &modifier.argument {
            shape.record_source_text(argument);
        }
        if let Some(word) = &modifier.argument_word_ast {
            visit_word(word, shape, 1);
        }
    }
    if let Some(operation) = &expansion.operation {
        visit_zsh_operation(operation, shape);
    }
}

fn visit_zsh_target(target: &ZshExpansionTarget, shape: &mut AstShape) {
    match target {
        ZshExpansionTarget::Reference(reference) => visit_var_ref(reference, shape),
        ZshExpansionTarget::Nested(parameter) => {
            shape.record_box();
            visit_parameter_expansion(parameter, shape);
        }
        ZshExpansionTarget::Word(word) => visit_word(word, shape, 1),
        ZshExpansionTarget::Empty => {}
    }
}

fn visit_zsh_operation(operation: &ZshExpansionOperation, shape: &mut AstShape) {
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
            shape.record_source_text(operand);
            visit_word(operand_word_ast, shape, 1);
        }
        ZshExpansionOperation::ReplacementOperation {
            pattern,
            pattern_word_ast,
            replacement,
            replacement_word_ast,
            ..
        } => {
            shape.record_source_text(pattern);
            visit_word(pattern_word_ast, shape, 1);
            if let Some(replacement) = replacement {
                shape.record_source_text(replacement);
            }
            if let Some(word) = replacement_word_ast {
                visit_word(word, shape, 1);
            }
        }
        ZshExpansionOperation::Slice {
            offset,
            offset_word_ast,
            length,
            length_word_ast,
        } => {
            shape.record_source_text(offset);
            visit_word(offset_word_ast, shape, 1);
            if let Some(length) = length {
                shape.record_source_text(length);
            }
            if let Some(word) = length_word_ast {
                visit_word(word, shape, 1);
            }
        }
        ZshExpansionOperation::Unknown { text, word_ast } => {
            shape.record_source_text(text);
            visit_word(word_ast, shape, 1);
        }
    }
}

fn visit_parameter_op(operator: &ParameterOp, shape: &mut AstShape) {
    match operator {
        ParameterOp::RemovePrefixShort { pattern }
        | ParameterOp::RemovePrefixLong { pattern }
        | ParameterOp::RemoveSuffixShort { pattern }
        | ParameterOp::RemoveSuffixLong { pattern } => visit_pattern(pattern, shape),
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
            visit_pattern(pattern, shape);
            shape.record_source_text(replacement);
            visit_word(replacement_word_ast, shape, 1);
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

fn visit_zsh_qualified_glob(glob: &ZshQualifiedGlob, shape: &mut AstShape) {
    shape.record_vec(&glob.segments);
    for segment in &glob.segments {
        match segment {
            ZshGlobSegment::Pattern(pattern) => visit_pattern(pattern, shape),
            ZshGlobSegment::InlineControl(_) => {}
        }
    }
    if let Some(qualifiers) = &glob.qualifiers {
        shape.record_vec(&qualifiers.fragments);
        for qualifier in &qualifiers.fragments {
            match qualifier {
                ZshGlobQualifier::LetterSequence { text, .. } => shape.record_source_text(text),
                ZshGlobQualifier::NumericArgument { start, end, .. } => {
                    shape.record_source_text(start);
                    if let Some(end) = end {
                        shape.record_source_text(end);
                    }
                }
                ZshGlobQualifier::Negation { .. } | ZshGlobQualifier::Flag { .. } => {}
            }
        }
    }
}

fn visit_pattern(pattern: &Pattern, shape: &mut AstShape) {
    shape.patterns += 1;
    shape.record_vec(&pattern.parts);
    for part in &pattern.parts {
        shape.pattern_parts += 1;
        match &part.kind {
            PatternPart::Literal(text) => shape.record_literal_text(text),
            PatternPart::AnyString | PatternPart::AnyChar => {}
            PatternPart::CharClass(text) => shape.record_source_text(text),
            PatternPart::Group { patterns, .. } => {
                shape.record_vec(patterns);
                for pattern in patterns {
                    visit_pattern(pattern, shape);
                }
            }
            PatternPart::Word(word) => visit_word(word, shape, 1),
        }
    }
}

fn visit_arithmetic_expr(expr: &ArithmeticExprNode, shape: &mut AstShape, depth: u64) {
    shape.arithmetic_exprs += 1;
    shape.max_arithmetic_depth = shape.max_arithmetic_depth.max(depth);
    match &expr.kind {
        ArithmeticExpr::Number(text) => shape.record_source_text(text),
        ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => {
            shape.record_box();
            visit_arithmetic_expr(index, shape, depth + 1);
        }
        ArithmeticExpr::ShellWord(word) => visit_word(word, shape, 1),
        ArithmeticExpr::Parenthesized { expression } => {
            shape.record_box();
            visit_arithmetic_expr(expression, shape, depth + 1);
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            shape.record_box();
            visit_arithmetic_expr(expr, shape, depth + 1);
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            shape.record_box();
            visit_arithmetic_expr(left, shape, depth + 1);
            shape.record_box();
            visit_arithmetic_expr(right, shape, depth + 1);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            shape.record_box();
            visit_arithmetic_expr(condition, shape, depth + 1);
            shape.record_box();
            visit_arithmetic_expr(then_expr, shape, depth + 1);
            shape.record_box();
            visit_arithmetic_expr(else_expr, shape, depth + 1);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            visit_arithmetic_lvalue(target, shape, depth + 1);
            shape.record_box();
            visit_arithmetic_expr(value, shape, depth + 1);
        }
    }
}

fn visit_arithmetic_lvalue(target: &ArithmeticLvalue, shape: &mut AstShape, depth: u64) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => {
            shape.record_box();
            visit_arithmetic_expr(index, shape, depth + 1);
        }
    }
}

fn visit_conditional_expr(expr: &ConditionalExpr, shape: &mut AstShape, depth: u64) {
    shape.conditional_exprs += 1;
    shape.max_conditional_depth = shape.max_conditional_depth.max(depth);
    match expr {
        ConditionalExpr::Binary(expr) => {
            shape.record_box();
            visit_conditional_expr(&expr.left, shape, depth + 1);
            shape.record_box();
            visit_conditional_expr(&expr.right, shape, depth + 1);
        }
        ConditionalExpr::Unary(expr) => {
            shape.record_box();
            visit_conditional_expr(&expr.expr, shape, depth + 1);
        }
        ConditionalExpr::Parenthesized(expr) => {
            shape.record_box();
            visit_conditional_expr(&expr.expr, shape, depth + 1);
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => visit_word(word, shape, 1),
        ConditionalExpr::Pattern(pattern) => visit_pattern(pattern, shape),
        ConditionalExpr::VarRef(reference) => {
            shape.record_box();
            visit_var_ref(reference, shape);
        }
    }
}

fn single_case_report(case_name: &str) -> Option<CaseReport> {
    let cases = benchmark_cases();
    let case = cases.into_iter().find(|case| case.name == case_name)?;
    let mut shape = AstShape::default();
    let mut recovered_files = 0usize;
    let mut command_count = 0usize;

    for file in case.files {
        let output = parse_fixture(file.source);
        recovered_files += usize::from(output.status != ParseStatus::Clean);
        command_count += output.file.body.len();
        visit_file(&output.file, &mut shape);
    }

    Some(CaseReport {
        case: case.name.to_string(),
        files: case.files.len(),
        recovered_files,
        command_count,
        estimated_replaceable_heap_allocations: shape.estimated_replaceable_heap_allocations(),
        shape,
    })
}

fn parse_case_arg() -> Option<String> {
    let mut args = std::env::args().skip(1);
    let arg = args.next()?;

    match arg.as_str() {
        "--case" => {
            let value = args.next();
            if let Some(extra) = args.next() {
                eprintln!("unknown argument `{extra}`");
                process::exit(2);
            }
            value
        }
        "--help" | "-h" => {
            eprintln!("usage: cargo run -p shuck-benchmark --example ast_shape -- [--case NAME]");
            process::exit(0);
        }
        _ => {
            eprintln!("unknown argument `{arg}`");
            process::exit(2);
        }
    }
}

fn main() -> serde_json::Result<()> {
    let requested_case = parse_case_arg();
    let reports = if let Some(case_name) = requested_case {
        let Some(report) = single_case_report(&case_name) else {
            eprintln!("unknown benchmark case `{case_name}`");
            process::exit(2);
        };
        vec![report]
    } else {
        benchmark_cases()
            .into_iter()
            .filter_map(|case| single_case_report(case.name))
            .collect::<Vec<_>>()
    };

    serde_json::to_writer_pretty(std::io::stdout().lock(), &reports)?;
    println!();
    Ok(())
}
