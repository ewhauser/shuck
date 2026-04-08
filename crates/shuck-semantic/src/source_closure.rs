use std::fs;
use std::path::{Path, PathBuf};

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, ArithmeticLvalue, ArrayElem, Assignment, AssignmentValue,
    BourneParameterExpansion, BuiltinCommand, Command, CompoundCommand, ConditionalExpr,
    DeclOperand, File, FunctionDef, Name, ParameterExpansion, ParameterExpansionSyntax, Pattern,
    PatternPart, PatternPartNode, Redirect, SourceText, Span, Stmt, StmtSeq, VarRef, Word,
    WordPart, WordPartNode, ZshExpansionOperation, ZshExpansionTarget,
};
use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;

use crate::{
    Binding, BindingId, ScopeId, ScopeKind, SemanticModel, SourcePathResolver, SourceRefKind,
    SpanKey, SyntheticRead,
};

pub(crate) fn collect_source_closure_reads(
    model: &SemanticModel,
    file: &File,
    source: &str,
    source_path: &Path,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> Vec<SyntheticRead> {
    let mut summaries = FxHashMap::default();
    let mut active = FxHashSet::default();
    collect_source_closure_reads_with_cache(
        model,
        file,
        source,
        source_path,
        &mut summaries,
        &mut active,
        source_path_resolver,
    )
}

fn collect_source_closure_reads_with_cache(
    model: &SemanticModel,
    file: &File,
    source: &str,
    source_path: &Path,
    summaries: &mut FxHashMap<PathBuf, FxHashSet<Name>>,
    active: &mut FxHashSet<PathBuf>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> Vec<SyntheticRead> {
    let facts = collect_ast_facts(file, model, source);
    let call_args_by_scope = resolve_literal_call_args_by_scope(model, &facts.calls);
    let mut seen = FxHashSet::default();
    let mut synthetic_reads = Vec::new();

    for source_ref in model.source_refs() {
        let scope = model.scope_at(source_ref.span.start.offset);
        let candidates = source_candidates(
            &source_ref.kind,
            facts.source_templates.get(&SpanKey::new(source_ref.span)),
            call_args_by_scope.get(&scope).map(Vec::as_slice),
            source_path,
        );

        extend_synthetic_reads_for_candidates(
            &mut synthetic_reads,
            &mut seen,
            scope,
            source_ref.span,
            source_path,
            candidates,
            summaries,
            active,
            source_path_resolver,
        );
    }

    for call in &facts.calls {
        let Some(candidate) = local_helper_command_candidate(&call.name) else {
            continue;
        };
        extend_synthetic_reads_for_candidates(
            &mut synthetic_reads,
            &mut seen,
            call.scope,
            call.span,
            source_path,
            [candidate],
            summaries,
            active,
            source_path_resolver,
        );
    }

    synthetic_reads
}

#[allow(clippy::too_many_arguments)]
fn extend_synthetic_reads_for_candidates(
    synthetic_reads: &mut Vec<SyntheticRead>,
    seen: &mut FxHashSet<(ScopeId, usize, Name)>,
    scope: ScopeId,
    span: Span,
    source_path: &Path,
    candidates: impl IntoIterator<Item = String>,
    summaries: &mut FxHashMap<PathBuf, FxHashSet<Name>>,
    active: &mut FxHashSet<PathBuf>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) {
    for candidate in candidates {
        for resolved_path in resolve_helper_paths(source_path, &candidate, source_path_resolver) {
            let reads = summarize_helper(&resolved_path, summaries, active, source_path_resolver);
            for name in reads {
                if seen.insert((scope, span.start.offset, name.clone())) {
                    synthetic_reads.push(SyntheticRead { scope, span, name });
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct AstFacts {
    source_templates: FxHashMap<SpanKey, SourcePathTemplate>,
    calls: Vec<CallInfo>,
}

#[derive(Debug, Clone)]
struct CallInfo {
    name: Name,
    scope: ScopeId,
    span: Span,
    args: Vec<Option<String>>,
}

#[derive(Debug, Clone)]
enum SourcePathTemplate {
    Interpolated(Vec<TemplatePart>),
}

#[derive(Debug, Clone)]
enum TemplatePart {
    Literal(String),
    Arg(usize),
    SourceDir,
    SourceFile,
}

fn collect_ast_facts(file: &File, model: &SemanticModel, source: &str) -> AstFacts {
    let mut facts = AstFacts {
        source_templates: FxHashMap::default(),
        calls: Vec::new(),
    };
    walk_stmt_seq(&file.body, model, source, &mut facts);
    facts
}

fn walk_stmt_seq(commands: &StmtSeq, model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    for stmt in commands.iter() {
        walk_stmt(stmt, model, source, facts);
    }
}

fn walk_stmt(stmt: &Stmt, model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    walk_redirects(&stmt.redirects, model, source, facts);
    walk_command(&stmt.command, model, source, facts);
}

fn walk_command(command: &Command, model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    match command {
        Command::Simple(command) => {
            if let Some(name) = static_word_text(&command.name, source)
                && !name.is_empty()
            {
                facts.calls.push(CallInfo {
                    name: Name::from(name.as_str()),
                    scope: model.scope_at(command.span.start.offset),
                    span: command.span,
                    args: command
                        .args
                        .iter()
                        .map(|word| static_word_text(word, source))
                        .collect(),
                });

                if matches!(name.as_str(), "source" | ".")
                    && let Some(argument) = command.args.first()
                    && let Some(template) =
                        source_path_template(argument, source, model.bash_runtime_vars_enabled())
                {
                    facts
                        .source_templates
                        .insert(SpanKey::new(command.span), template);
                }
            }

            walk_assignments(&command.assignments, model, source, facts);
            walk_word(&command.name, model, source, facts);
            walk_words(&command.args, model, source, facts);
        }
        Command::Builtin(BuiltinCommand::Break(command)) => {
            walk_assignments(&command.assignments, model, source, facts);
            if let Some(word) = &command.depth {
                walk_word(word, model, source, facts);
            }
            walk_words(&command.extra_args, model, source, facts);
        }
        Command::Builtin(BuiltinCommand::Continue(command)) => {
            walk_assignments(&command.assignments, model, source, facts);
            if let Some(word) = &command.depth {
                walk_word(word, model, source, facts);
            }
            walk_words(&command.extra_args, model, source, facts);
        }
        Command::Builtin(BuiltinCommand::Return(command)) => {
            walk_assignments(&command.assignments, model, source, facts);
            if let Some(word) = &command.code {
                walk_word(word, model, source, facts);
            }
            walk_words(&command.extra_args, model, source, facts);
        }
        Command::Builtin(BuiltinCommand::Exit(command)) => {
            walk_assignments(&command.assignments, model, source, facts);
            if let Some(word) = &command.code {
                walk_word(word, model, source, facts);
            }
            walk_words(&command.extra_args, model, source, facts);
        }
        Command::Decl(command) => {
            walk_assignments(&command.assignments, model, source, facts);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        walk_word(word, model, source, facts);
                    }
                    DeclOperand::Name(name) => walk_var_ref_subscript(name, model, source, facts),
                    DeclOperand::Assignment(assignment) => {
                        walk_assignment(assignment, model, source, facts);
                    }
                }
            }
        }
        Command::Binary(command) => {
            walk_stmt(&command.left, model, source, facts);
            walk_stmt(&command.right, model, source, facts);
        }
        Command::Compound(command) => {
            walk_compound(command, model, source, facts);
        }
        Command::Function(FunctionDef { body, .. }) => walk_stmt(body, model, source, facts),
    }
}

fn walk_compound(
    command: &CompoundCommand,
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    match command {
        CompoundCommand::If(command) => {
            walk_stmt_seq(&command.condition, model, source, facts);
            walk_stmt_seq(&command.then_branch, model, source, facts);
            for (condition, body) in &command.elif_branches {
                walk_stmt_seq(condition, model, source, facts);
                walk_stmt_seq(body, model, source, facts);
            }
            if let Some(body) = &command.else_branch {
                walk_stmt_seq(body, model, source, facts);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                walk_words(words, model, source, facts);
            }
            walk_stmt_seq(&command.body, model, source, facts);
        }
        CompoundCommand::Repeat(command) => {
            walk_word(&command.count, model, source, facts);
            walk_stmt_seq(&command.body, model, source, facts);
        }
        CompoundCommand::Foreach(command) => {
            walk_words(&command.words, model, source, facts);
            walk_stmt_seq(&command.body, model, source, facts);
        }
        CompoundCommand::ArithmeticFor(command) => {
            if let Some(expr) = &command.init_ast {
                walk_arithmetic_expr(expr, model, source, facts);
            }
            if let Some(expr) = &command.condition_ast {
                walk_arithmetic_expr(expr, model, source, facts);
            }
            if let Some(expr) = &command.step_ast {
                walk_arithmetic_expr(expr, model, source, facts);
            }
            walk_stmt_seq(&command.body, model, source, facts)
        }
        CompoundCommand::While(command) => {
            walk_stmt_seq(&command.condition, model, source, facts);
            walk_stmt_seq(&command.body, model, source, facts);
        }
        CompoundCommand::Until(command) => {
            walk_stmt_seq(&command.condition, model, source, facts);
            walk_stmt_seq(&command.body, model, source, facts);
        }
        CompoundCommand::Case(command) => {
            walk_word(&command.word, model, source, facts);
            for case in &command.cases {
                walk_patterns(&case.patterns, model, source, facts);
                walk_stmt_seq(&case.body, model, source, facts);
            }
        }
        CompoundCommand::Select(command) => {
            walk_words(&command.words, model, source, facts);
            walk_stmt_seq(&command.body, model, source, facts);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            walk_stmt_seq(commands, model, source, facts);
        }
        CompoundCommand::Always(command) => {
            walk_stmt_seq(&command.body, model, source, facts);
            walk_stmt_seq(&command.always_body, model, source, facts);
        }
        CompoundCommand::Arithmetic(command) => {
            if let Some(expr) = &command.expr_ast {
                walk_arithmetic_expr(expr, model, source, facts);
            }
        }
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                walk_stmt(command, model, source, facts);
            }
        }
        CompoundCommand::Conditional(command) => {
            walk_conditional_expr(&command.expression, model, source, facts)
        }
        CompoundCommand::Coproc(command) => walk_stmt(&command.body, model, source, facts),
    }
}

fn walk_assignments(
    assignments: &[Assignment],
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    for assignment in assignments {
        walk_assignment(assignment, model, source, facts);
    }
}

fn walk_assignment(
    assignment: &Assignment,
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    walk_var_ref_subscript(&assignment.target, model, source, facts);
    match &assignment.value {
        AssignmentValue::Scalar(word) => walk_word(word, model, source, facts),
        AssignmentValue::Compound(array) => {
            for element in &array.elements {
                match element {
                    ArrayElem::Sequential(word) => walk_word(word, model, source, facts),
                    ArrayElem::Keyed { key, value } | ArrayElem::KeyedAppend { key, value } => {
                        walk_subscript(Some(key), model, source, facts);
                        walk_word(value, model, source, facts);
                    }
                }
            }
        }
    }
}

fn walk_words(words: &[Word], model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    for word in words {
        walk_word(word, model, source, facts);
    }
}

fn walk_patterns(patterns: &[Pattern], model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    for pattern in patterns {
        walk_pattern(pattern, model, source, facts);
    }
}

fn walk_word(word: &Word, model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    walk_word_parts(&word.parts, model, source, facts);
}

fn walk_pattern(pattern: &Pattern, model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    walk_pattern_parts(&pattern.parts, model, source, facts);
}

fn walk_word_parts(
    parts: &[WordPartNode],
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    for part in parts {
        match &part.kind {
            WordPart::SingleQuoted { .. } => {}
            WordPart::DoubleQuoted { parts, .. } => walk_word_parts(parts, model, source, facts),
            WordPart::CommandSubstitution { body, .. }
            | WordPart::ProcessSubstitution { body, .. } => {
                walk_stmt_seq(body, model, source, facts)
            }
            WordPart::ArithmeticExpansion { expression_ast, .. } => {
                if let Some(expr) = expression_ast {
                    walk_arithmetic_expr(expr, model, source, facts);
                }
            }
            WordPart::Parameter(parameter) => {
                walk_parameter_expansion(parameter, model, source, facts);
            }
            WordPart::ParameterExpansion {
                reference,
                operator,
                operand,
                ..
            } => {
                walk_var_ref_subscript(reference, model, source, facts);
                if let Some(operand) = operand {
                    walk_source_text(operand, model, source, facts);
                }
                match operator {
                    shuck_ast::ParameterOp::RemovePrefixShort { pattern }
                    | shuck_ast::ParameterOp::RemovePrefixLong { pattern }
                    | shuck_ast::ParameterOp::RemoveSuffixShort { pattern }
                    | shuck_ast::ParameterOp::RemoveSuffixLong { pattern }
                    | shuck_ast::ParameterOp::ReplaceFirst { pattern, .. }
                    | shuck_ast::ParameterOp::ReplaceAll { pattern, .. } => {
                        walk_pattern(pattern, model, source, facts);
                    }
                    shuck_ast::ParameterOp::UseDefault
                    | shuck_ast::ParameterOp::AssignDefault
                    | shuck_ast::ParameterOp::UseReplacement
                    | shuck_ast::ParameterOp::Error
                    | shuck_ast::ParameterOp::UpperFirst
                    | shuck_ast::ParameterOp::UpperAll
                    | shuck_ast::ParameterOp::LowerFirst
                    | shuck_ast::ParameterOp::LowerAll => {}
                }
            }
            WordPart::Substring {
                reference,
                offset_ast,
                length_ast,
                ..
            }
            | WordPart::ArraySlice {
                reference,
                offset_ast,
                length_ast,
                ..
            } => {
                walk_var_ref_subscript(reference, model, source, facts);
                if let Some(offset_ast) = offset_ast {
                    walk_arithmetic_expr(offset_ast, model, source, facts);
                }
                if let Some(length_ast) = length_ast {
                    walk_arithmetic_expr(length_ast, model, source, facts);
                }
            }
            WordPart::ArrayAccess(reference) => {
                walk_var_ref_subscript(reference, model, source, facts);
            }
            WordPart::IndirectExpansion { operand, .. } => {
                if let Some(operand) = operand {
                    walk_source_text(operand, model, source, facts);
                }
            }
            WordPart::Transformation { reference, .. }
            | WordPart::Length(reference)
            | WordPart::ArrayLength(reference)
            | WordPart::ArrayIndices(reference) => {
                walk_var_ref_subscript(reference, model, source, facts);
            }
            WordPart::Literal(_) | WordPart::Variable(_) | WordPart::PrefixMatch { .. } => {}
        }
    }
}

fn walk_parameter_expansion(
    parameter: &ParameterExpansion,
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(syntax) => match syntax {
            BourneParameterExpansion::Access { reference }
            | BourneParameterExpansion::Length { reference }
            | BourneParameterExpansion::Indices { reference }
            | BourneParameterExpansion::Transformation { reference, .. } => {
                walk_var_ref_subscript(reference, model, source, facts);
            }
            BourneParameterExpansion::Indirect { operand, .. } => {
                if let Some(operand) = operand {
                    walk_source_text(operand, model, source, facts);
                }
            }
            BourneParameterExpansion::PrefixMatch { .. } => {}
            BourneParameterExpansion::Slice {
                reference,
                offset_ast,
                length_ast,
                ..
            } => {
                walk_var_ref_subscript(reference, model, source, facts);
                if let Some(offset_ast) = offset_ast {
                    walk_arithmetic_expr(offset_ast, model, source, facts);
                }
                if let Some(length_ast) = length_ast {
                    walk_arithmetic_expr(length_ast, model, source, facts);
                }
            }
            BourneParameterExpansion::Operation {
                reference,
                operator,
                operand,
                ..
            } => {
                walk_var_ref_subscript(reference, model, source, facts);
                if let Some(operand) = operand {
                    walk_source_text(operand, model, source, facts);
                }
                match operator {
                    shuck_ast::ParameterOp::RemovePrefixShort { pattern }
                    | shuck_ast::ParameterOp::RemovePrefixLong { pattern }
                    | shuck_ast::ParameterOp::RemoveSuffixShort { pattern }
                    | shuck_ast::ParameterOp::RemoveSuffixLong { pattern }
                    | shuck_ast::ParameterOp::ReplaceFirst { pattern, .. }
                    | shuck_ast::ParameterOp::ReplaceAll { pattern, .. } => {
                        walk_pattern(pattern, model, source, facts);
                    }
                    shuck_ast::ParameterOp::UseDefault
                    | shuck_ast::ParameterOp::AssignDefault
                    | shuck_ast::ParameterOp::UseReplacement
                    | shuck_ast::ParameterOp::Error
                    | shuck_ast::ParameterOp::UpperFirst
                    | shuck_ast::ParameterOp::UpperAll
                    | shuck_ast::ParameterOp::LowerFirst
                    | shuck_ast::ParameterOp::LowerAll => {}
                }
            }
        },
        ParameterExpansionSyntax::Zsh(syntax) => {
            match &syntax.target {
                ZshExpansionTarget::Reference(reference) => {
                    walk_var_ref_subscript(reference, model, source, facts);
                }
                ZshExpansionTarget::Nested(parameter) => {
                    walk_parameter_expansion(parameter, model, source, facts);
                }
                ZshExpansionTarget::Empty => {}
            }

            for modifier in &syntax.modifiers {
                if let Some(argument) = &modifier.argument {
                    walk_source_text(argument, model, source, facts);
                }
            }

            if let Some(operation) = &syntax.operation {
                match operation {
                    ZshExpansionOperation::PatternOperation { operand, .. }
                    | ZshExpansionOperation::Defaulting { operand, .. }
                    | ZshExpansionOperation::TrimOperation { operand, .. }
                    | ZshExpansionOperation::Unknown(operand) => {
                        walk_source_text(operand, model, source, facts);
                    }
                    ZshExpansionOperation::ReplacementOperation {
                        pattern,
                        replacement,
                        ..
                    } => {
                        walk_source_text(pattern, model, source, facts);
                        if let Some(replacement) = replacement {
                            walk_source_text(replacement, model, source, facts);
                        }
                    }
                    ZshExpansionOperation::Slice { offset, length } => {
                        walk_source_text(offset, model, source, facts);
                        if let Some(length) = length {
                            walk_source_text(length, model, source, facts);
                        }
                    }
                }
            }
        }
    }
}

fn walk_arithmetic_expr(
    expr: &ArithmeticExprNode,
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    match &expr.kind {
        ArithmeticExpr::Number(_) | ArithmeticExpr::Variable(_) => {}
        ArithmeticExpr::Indexed { index, .. } => walk_arithmetic_expr(index, model, source, facts),
        ArithmeticExpr::ShellWord(word) => walk_word(word, model, source, facts),
        ArithmeticExpr::Parenthesized { expression } => {
            walk_arithmetic_expr(expression, model, source, facts)
        }
        ArithmeticExpr::Unary { expr, .. } | ArithmeticExpr::Postfix { expr, .. } => {
            walk_arithmetic_expr(expr, model, source, facts)
        }
        ArithmeticExpr::Binary { left, right, .. } => {
            walk_arithmetic_expr(left, model, source, facts);
            walk_arithmetic_expr(right, model, source, facts);
        }
        ArithmeticExpr::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            walk_arithmetic_expr(condition, model, source, facts);
            walk_arithmetic_expr(then_expr, model, source, facts);
            walk_arithmetic_expr(else_expr, model, source, facts);
        }
        ArithmeticExpr::Assignment { target, value, .. } => {
            walk_arithmetic_lvalue(target, model, source, facts);
            walk_arithmetic_expr(value, model, source, facts);
        }
    }
}

fn walk_arithmetic_lvalue(
    target: &ArithmeticLvalue,
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    match target {
        ArithmeticLvalue::Variable(_) => {}
        ArithmeticLvalue::Indexed { index, .. } => {
            walk_arithmetic_expr(index, model, source, facts)
        }
    }
}

fn walk_pattern_parts(
    parts: &[PatternPartNode],
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    for part in parts {
        match &part.kind {
            PatternPart::Group { patterns, .. } => walk_patterns(patterns, model, source, facts),
            PatternPart::Word(word) => walk_word(word, model, source, facts),
            PatternPart::Literal(_)
            | PatternPart::AnyString
            | PatternPart::AnyChar
            | PatternPart::CharClass(_) => {}
        }
    }
}

fn walk_source_text(text: &SourceText, model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    let word = Parser::parse_word_fragment(source, text.slice(source), text.span());
    walk_word(&word, model, source, facts);
}

fn walk_redirects(
    redirects: &[Redirect],
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    for redirect in redirects {
        let word = match redirect.word_target() {
            Some(word) => word,
            None => &redirect.heredoc().expect("expected heredoc redirect").body,
        };
        walk_word(word, model, source, facts);
    }
}

fn walk_conditional_expr(
    expression: &ConditionalExpr,
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    match expression {
        ConditionalExpr::Binary(expr) => {
            walk_conditional_expr(&expr.left, model, source, facts);
            walk_conditional_expr(&expr.right, model, source, facts);
        }
        ConditionalExpr::Unary(expr) => walk_conditional_expr(&expr.expr, model, source, facts),
        ConditionalExpr::Parenthesized(expr) => {
            walk_conditional_expr(&expr.expr, model, source, facts)
        }
        ConditionalExpr::Word(word) | ConditionalExpr::Regex(word) => {
            walk_word(word, model, source, facts)
        }
        ConditionalExpr::Pattern(pattern) => walk_pattern(pattern, model, source, facts),
        ConditionalExpr::VarRef(var_ref) => walk_var_ref_subscript(var_ref, model, source, facts),
    }
}

fn walk_var_ref_subscript(
    reference: &VarRef,
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    walk_subscript(reference.subscript.as_ref(), model, source, facts);
}

fn walk_subscript(
    subscript: Option<&shuck_ast::Subscript>,
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    let Some(subscript) = subscript else {
        return;
    };
    if subscript.selector().is_some() {
        return;
    }
    if let Some(expr) = subscript.arithmetic_ast.as_ref() {
        walk_arithmetic_expr(expr, model, source, facts);
        return;
    }

    walk_source_text(subscript.syntax_source_text(), model, source, facts);
}

fn source_path_template(
    word: &Word,
    source: &str,
    bash_runtime_vars_enabled: bool,
) -> Option<SourcePathTemplate> {
    if static_word_text(word, source).is_some() {
        return None;
    }

    let mut parts = Vec::new();
    let mut ignored_root = false;
    let mut saw_dynamic = false;

    if !collect_source_template_parts(
        &word.parts,
        source,
        bash_runtime_vars_enabled,
        &mut parts,
        &mut ignored_root,
        &mut saw_dynamic,
    ) {
        return None;
    }

    (saw_dynamic && !parts.is_empty()).then_some(SourcePathTemplate::Interpolated(parts))
}

fn collect_source_template_parts(
    word_parts: &[WordPartNode],
    source: &str,
    bash_runtime_vars_enabled: bool,
    parts: &mut Vec<TemplatePart>,
    ignored_root: &mut bool,
    saw_dynamic: &mut bool,
) -> bool {
    for part in word_parts {
        match &part.kind {
            WordPart::Literal(text) => {
                let text = text.as_str(source, part.span);
                if !text.is_empty() {
                    push_literal(parts, text.to_owned());
                }
            }
            WordPart::SingleQuoted { value, .. } => {
                let text = value.slice(source);
                if !text.is_empty() {
                    push_literal(parts, text.to_owned());
                }
            }
            WordPart::DoubleQuoted { parts: inner, .. } => {
                if !collect_source_template_parts(
                    inner,
                    source,
                    bash_runtime_vars_enabled,
                    parts,
                    ignored_root,
                    saw_dynamic,
                ) {
                    return false;
                }
            }
            WordPart::Variable(name) => {
                if let Some(index) = positional_index(name) {
                    *saw_dynamic = true;
                    parts.push(TemplatePart::Arg(index));
                } else if bash_runtime_vars_enabled && is_bash_source_var(name) {
                    *saw_dynamic = true;
                    parts.push(TemplatePart::SourceFile);
                } else if !*ignored_root && parts.is_empty() {
                    *ignored_root = true;
                    *saw_dynamic = true;
                } else {
                    return false;
                }
            }
            WordPart::Parameter(parameter)
                if bash_runtime_vars_enabled
                    && parameter_is_current_source_file(parameter, source) =>
            {
                *saw_dynamic = true;
                parts.push(TemplatePart::SourceFile);
            }
            WordPart::ArrayAccess(reference)
                if bash_runtime_vars_enabled && is_bash_source_index_ref(reference, source) =>
            {
                *saw_dynamic = true;
                parts.push(TemplatePart::SourceFile);
            }
            WordPart::CommandSubstitution { body, .. } => {
                if bash_runtime_vars_enabled
                    && let Some(template_part) = dirname_source_template_part(body, source)
                {
                    *saw_dynamic = true;
                    parts.push(template_part);
                } else {
                    return false;
                }
            }
            _ => return false,
        }
    }

    true
}

fn push_literal(parts: &mut Vec<TemplatePart>, text: String) {
    if let Some(TemplatePart::Literal(existing)) = parts.last_mut() {
        existing.push_str(&text);
    } else {
        parts.push(TemplatePart::Literal(text));
    }
}

fn positional_index(name: &Name) -> Option<usize> {
    name.as_str().parse().ok()
}

fn is_bash_source_var(name: &Name) -> bool {
    name.as_str() == "BASH_SOURCE"
}

fn parameter_is_current_source_file(parameter: &ParameterExpansion, source: &str) -> bool {
    match &parameter.syntax {
        ParameterExpansionSyntax::Bourne(BourneParameterExpansion::Access { reference }) => {
            is_current_source_reference(reference, source)
        }
        ParameterExpansionSyntax::Bourne(
            BourneParameterExpansion::Length { .. }
            | BourneParameterExpansion::Indices { .. }
            | BourneParameterExpansion::Indirect { .. }
            | BourneParameterExpansion::PrefixMatch { .. }
            | BourneParameterExpansion::Slice { .. }
            | BourneParameterExpansion::Operation { .. }
            | BourneParameterExpansion::Transformation { .. },
        )
        | ParameterExpansionSyntax::Zsh(_) => false,
    }
}

fn is_current_source_reference(reference: &VarRef, source: &str) -> bool {
    is_bash_source_var(&reference.name)
        && reference
            .subscript
            .as_ref()
            .is_none_or(|subscript| subscript_is_semantic_zero(subscript, source))
}

fn is_bash_source_index_ref(reference: &VarRef, source: &str) -> bool {
    is_bash_source_var(&reference.name)
        && reference
            .subscript
            .as_ref()
            .is_some_and(|subscript| subscript_is_semantic_zero(subscript, source))
}

fn subscript_is_semantic_zero(subscript: &shuck_ast::Subscript, source: &str) -> bool {
    subscript
        .arithmetic_ast
        .as_ref()
        .is_some_and(|expr| arithmetic_expr_is_semantic_zero(expr, source))
}

fn arithmetic_expr_is_semantic_zero(expr: &ArithmeticExprNode, source: &str) -> bool {
    match &expr.kind {
        ArithmeticExpr::Number(text) => shell_zero_literal(text.slice(source)),
        ArithmeticExpr::ShellWord(word) => word_is_semantic_zero(word, source),
        ArithmeticExpr::Parenthesized { expression } => {
            arithmetic_expr_is_semantic_zero(expression, source)
        }
        ArithmeticExpr::Unary { expr, .. } => arithmetic_expr_is_semantic_zero(expr, source),
        _ => false,
    }
}

fn word_is_semantic_zero(word: &Word, source: &str) -> bool {
    matches!(
        word.parts.as_slice(),
        [part] if match &part.kind {
            WordPart::Literal(text) => shell_zero_literal(text.as_str(source, part.span)),
            WordPart::SingleQuoted { value, .. } => shell_zero_literal(value.slice(source)),
            WordPart::DoubleQuoted { parts, .. } => matches!(
                parts.as_slice(),
                [part] if word_part_is_semantic_zero(&part.kind, part.span, source)
            ),
            WordPart::ArithmeticExpansion {
                expression_ast: Some(expr),
                ..
            } => arithmetic_expr_is_semantic_zero(expr, source),
            _ => false,
        }
    )
}

fn word_part_is_semantic_zero(part: &WordPart, span: Span, source: &str) -> bool {
    match part {
        WordPart::Literal(text) => shell_zero_literal(text.as_str(source, span)),
        WordPart::SingleQuoted { value, .. } => shell_zero_literal(value.slice(source)),
        WordPart::DoubleQuoted { parts, .. } => matches!(
            parts.as_slice(),
            [part] if word_part_is_semantic_zero(&part.kind, part.span, source)
        ),
        WordPart::ArithmeticExpansion {
            expression_ast: Some(expr),
            ..
        } => arithmetic_expr_is_semantic_zero(expr, source),
        _ => false,
    }
}

fn shell_zero_literal(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }

    let digits = text
        .strip_prefix('+')
        .or_else(|| text.strip_prefix('-'))
        .unwrap_or(text);
    if digits.is_empty() {
        return false;
    }

    if let Some((base, value)) = digits.split_once('#') {
        return base.parse::<u32>().is_ok_and(|base| {
            (2..=64).contains(&base) && !value.is_empty() && value.chars().all(|ch| ch == '0')
        });
    }

    let digits = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
        .unwrap_or(digits);
    !digits.is_empty() && digits.chars().all(|ch| ch == '0')
}

fn dirname_source_template_part(commands: &StmtSeq, source: &str) -> Option<TemplatePart> {
    let [stmt] = commands.as_slice() else {
        return None;
    };
    let Command::Simple(command) = &stmt.command else {
        return None;
    };
    if stmt.negated
        || !stmt.redirects.is_empty()
        || !command.assignments.is_empty()
        || command.args.len() != 1
    {
        return None;
    }
    if static_word_text(&command.name, source).as_deref() != Some("dirname") {
        return None;
    }
    current_source_file_word(&command.args[0], source).then_some(TemplatePart::SourceDir)
}

fn current_source_file_word(word: &Word, source: &str) -> bool {
    matches!(
        word.parts.as_slice(),
        [part] if is_current_source_part(&part.kind, source)
    )
}

fn is_current_source_part(part: &WordPart, source: &str) -> bool {
    match part {
        WordPart::Variable(name) => is_bash_source_var(name),
        WordPart::Parameter(parameter) => parameter_is_current_source_file(parameter, source),
        WordPart::ArrayAccess(reference) => is_bash_source_index_ref(reference, source),
        WordPart::DoubleQuoted { parts, .. } => {
            matches!(parts.as_slice(), [part] if is_current_source_part(&part.kind, source))
        }
        _ => false,
    }
}

fn source_candidates(
    kind: &SourceRefKind,
    template: Option<&SourcePathTemplate>,
    call_args: Option<&[Vec<Option<String>>]>,
    source_path: &Path,
) -> Vec<String> {
    match kind {
        SourceRefKind::DirectiveDevNull => Vec::new(),
        SourceRefKind::Literal(path) | SourceRefKind::Directive(path) => vec![path.clone()],
        SourceRefKind::Dynamic | SourceRefKind::SingleVariableStaticTail { .. } => {
            source_candidates_from_template(template, call_args, source_path)
        }
    }
}

fn source_candidates_from_template(
    template: Option<&SourcePathTemplate>,
    call_args: Option<&[Vec<Option<String>>]>,
    source_path: &Path,
) -> Vec<String> {
    let Some(template) = template else {
        return Vec::new();
    };

    match template {
        SourcePathTemplate::Interpolated(parts) => {
            if uses_positional_args(parts) {
                call_args
                    .into_iter()
                    .flatten()
                    .filter_map(|args| render_template_candidate(parts, args, source_path))
                    .collect()
            } else {
                render_template_candidate(parts, &[], source_path)
                    .into_iter()
                    .collect()
            }
        }
    }
}

fn local_helper_command_candidate(name: &Name) -> Option<String> {
    let name = name.as_str();
    // Treat sibling shell-script invocations like helper reads so globals used
    // across a script suite stay live, matching the large-corpus compatibility
    // expectation for module-style shell projects.
    (!matches!(name, "source" | ".") && looks_like_local_helper_command(name))
        .then(|| name.to_owned())
}

fn looks_like_local_helper_command(name: &str) -> bool {
    name.contains('/') || name.ends_with(".sh")
}

fn uses_positional_args(parts: &[TemplatePart]) -> bool {
    parts
        .iter()
        .any(|part| matches!(part, TemplatePart::Arg(_)))
}

fn render_template_candidate(
    parts: &[TemplatePart],
    args: &[Option<String>],
    source_path: &Path,
) -> Option<String> {
    let mut rendered = String::new();
    for part in parts {
        match part {
            TemplatePart::Literal(text) => rendered.push_str(text),
            TemplatePart::Arg(index) => {
                let value = args.get(index.saturating_sub(1))?.as_ref()?;
                rendered.push_str(value);
            }
            TemplatePart::SourceDir => {
                let value = source_path.parent()?.to_string_lossy();
                rendered.push_str(&value);
            }
            TemplatePart::SourceFile => {
                let value = source_path.to_string_lossy();
                rendered.push_str(&value);
            }
        }
    }

    let trimmed = rendered.trim();
    if trimmed.is_empty() {
        return None;
    }

    let source_derived = parts
        .iter()
        .any(|part| matches!(part, TemplatePart::SourceDir | TemplatePart::SourceFile));
    if source_derived && Path::new(trimmed).is_absolute() {
        return Some(trimmed.to_owned());
    }

    let normalized = trimmed
        .trim_start_matches("./")
        .trim_start_matches('/')
        .to_owned();
    (!normalized.is_empty()).then_some(normalized)
}

fn resolve_literal_call_args_by_scope(
    model: &SemanticModel,
    calls: &[CallInfo],
) -> FxHashMap<ScopeId, Vec<Vec<Option<String>>>> {
    let function_scopes = function_scopes_by_binding(model.scopes(), model.bindings());
    let mut resolved = FxHashMap::default();

    for call in calls {
        let Some(function_binding) =
            visible_function_binding(model, &call.name, call.scope, call.span.start.offset)
        else {
            continue;
        };
        let Some(callee_scope) = function_scopes.get(&function_binding).copied() else {
            continue;
        };
        resolved
            .entry(callee_scope)
            .or_insert_with(Vec::new)
            .push(call.args.clone());
    }

    resolved
}

fn function_scopes_by_binding(
    scopes: &[crate::Scope],
    bindings: &[Binding],
) -> FxHashMap<BindingId, ScopeId> {
    let mut bindings_by_parent_and_name: FxHashMap<(ScopeId, Name), Vec<BindingId>> =
        FxHashMap::default();
    for binding in bindings {
        if matches!(binding.kind, crate::BindingKind::FunctionDefinition) {
            bindings_by_parent_and_name
                .entry((binding.scope, binding.name.clone()))
                .or_default()
                .push(binding.id);
        }
    }
    for binding_ids in bindings_by_parent_and_name.values_mut() {
        binding_ids.sort_by_key(|binding| bindings[binding.index()].span.start.offset);
    }

    let mut scopes_by_parent_and_name: FxHashMap<(ScopeId, Name), Vec<ScopeId>> =
        FxHashMap::default();
    for scope in scopes {
        if let ScopeKind::Function(name) = &scope.kind
            && let Some(parent) = scope.parent
        {
            scopes_by_parent_and_name
                .entry((parent, name.clone()))
                .or_default()
                .push(scope.id);
        }
    }
    for scope_ids in scopes_by_parent_and_name.values_mut() {
        scope_ids.sort_by_key(|scope| scopes[scope.index()].span.start.offset);
    }

    let mut function_scopes = FxHashMap::default();
    for (key, binding_ids) in bindings_by_parent_and_name {
        let Some(scope_ids) = scopes_by_parent_and_name.get(&key) else {
            continue;
        };
        for (binding_id, scope_id) in binding_ids.into_iter().zip(scope_ids.iter().copied()) {
            function_scopes.insert(binding_id, scope_id);
        }
    }
    function_scopes
}

fn visible_function_binding(
    model: &SemanticModel,
    name: &Name,
    scope: ScopeId,
    offset: usize,
) -> Option<BindingId> {
    for scope_id in model.ancestor_scopes(scope) {
        let Some(candidates) = model.scopes()[scope_id.index()].bindings.get(name) else {
            continue;
        };
        for binding in candidates.iter().rev().copied() {
            let candidate = model.binding(binding);
            if matches!(candidate.kind, crate::BindingKind::FunctionDefinition)
                && candidate.span.start.offset <= offset
            {
                return Some(binding);
            }
        }
    }
    None
}

fn resolve_helper_paths(
    source_path: &Path,
    candidate: &str,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> Vec<PathBuf> {
    let candidate_path = Path::new(candidate);
    if candidate_path.is_absolute() {
        return candidate_path
            .is_file()
            .then_some(candidate_path.to_path_buf())
            .into_iter()
            .collect();
    }

    let Some(base_dir) = source_path.parent() else {
        return Vec::new();
    };

    let direct = base_dir.join(candidate_path);
    if direct.is_file() {
        return vec![direct];
    }

    source_path_resolver
        .into_iter()
        .flat_map(|resolver| resolver.resolve_candidate_paths(source_path, candidate))
        .filter(|path| path.is_file())
        .collect()
}

fn summarize_helper(
    path: &Path,
    summaries: &mut FxHashMap<PathBuf, FxHashSet<Name>>,
    active: &mut FxHashSet<PathBuf>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> FxHashSet<Name> {
    let key = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(summary) = summaries.get(&key) {
        return summary.clone();
    }
    if !active.insert(key.clone()) {
        return FxHashSet::default();
    }

    let summary = summarize_helper_uncached(&key, summaries, active, source_path_resolver);
    active.remove(&key);
    summaries.insert(key, summary.clone());
    summary
}

fn summarize_helper_uncached(
    path: &Path,
    summaries: &mut FxHashMap<PathBuf, FxHashSet<Name>>,
    active: &mut FxHashSet<PathBuf>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> FxHashSet<Name> {
    let Ok(source) = fs::read_to_string(path) else {
        return FxHashSet::default();
    };
    let Ok(output) = Parser::new(&source).parse() else {
        return FxHashSet::default();
    };
    let indexer = Indexer::new(&source, &output);
    let mut observer = crate::NoopTraversalObserver;
    let semantic = crate::build_semantic_model(
        &output.file,
        &source,
        &indexer,
        &mut observer,
        Some(path),
        false,
        source_path_resolver,
    );

    let mut reads = semantic
        .unresolved_references()
        .iter()
        .map(|reference| semantic.reference(*reference).name.clone())
        .collect::<FxHashSet<_>>();
    reads.extend(
        collect_source_closure_reads_with_cache(
            &semantic,
            &output.file,
            &source,
            path,
            summaries,
            active,
            source_path_resolver,
        )
        .into_iter()
        .map(|read| read.name),
    );
    reads
}

fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    collect_static_word_text(&word.parts, source, &mut result).then_some(result)
}

fn collect_static_word_text(parts: &[WordPartNode], source: &str, out: &mut String) -> bool {
    for part in parts {
        match &part.kind {
            WordPart::Literal(text) => out.push_str(text.as_str(source, part.span)),
            WordPart::SingleQuoted { value, .. } => out.push_str(value.slice(source)),
            WordPart::DoubleQuoted { parts, .. } => {
                if !collect_static_word_text(parts, source, out) {
                    return false;
                }
            }
            _ => return false,
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_parser::parser::ShellDialect;

    #[test]
    fn zsh_operation_operands_are_walked_when_collecting_ast_facts() {
        let source = "print ${(m)foo#$(printf '%s' \"$needle\")} ${(S)foo/$pattern/$(dirname \"$1\")} ${(m)foo:$(source \"$2\"):${length}}\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build(&output.file, source, &indexer);
        let facts = collect_ast_facts(&output.file, &model, source);
        let call_names = facts
            .calls
            .iter()
            .map(|call| call.name.to_string())
            .collect::<Vec<_>>();

        assert!(call_names.iter().any(|name| name == "printf"));
        assert!(call_names.iter().any(|name| name == "dirname"));
        assert!(call_names.iter().any(|name| name == "source"));
    }
}
