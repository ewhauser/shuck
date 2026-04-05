use std::fs;
use std::path::{Path, PathBuf};

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    Assignment, AssignmentValue, BuiltinCommand, Command, CompoundCommand, ConditionalExpr,
    DeclOperand, FunctionDef, Name, Redirect, Script, SourceText, Span, Word, WordPart,
};
use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;

use crate::{
    Binding, BindingId, ScopeId, ScopeKind, SemanticModel, SourceRefKind, SpanKey, SyntheticRead,
};

pub(crate) fn collect_source_closure_reads(
    model: &SemanticModel,
    script: &Script,
    source: &str,
    source_path: &Path,
) -> Vec<SyntheticRead> {
    let mut summaries = FxHashMap::default();
    let mut active = FxHashSet::default();
    collect_source_closure_reads_with_cache(
        model,
        script,
        source,
        source_path,
        &mut summaries,
        &mut active,
    )
}

fn collect_source_closure_reads_with_cache(
    model: &SemanticModel,
    script: &Script,
    source: &str,
    source_path: &Path,
    summaries: &mut FxHashMap<PathBuf, FxHashSet<Name>>,
    active: &mut FxHashSet<PathBuf>,
) -> Vec<SyntheticRead> {
    let facts = collect_ast_facts(script, model, source);
    let call_args_by_scope = resolve_literal_call_args_by_scope(model, &facts.calls);
    let mut seen = FxHashSet::default();
    let mut synthetic_reads = Vec::new();

    for source_ref in model.source_refs() {
        let scope = model.scope_at(source_ref.span.start.offset);
        let candidates = source_candidates(
            &source_ref.kind,
            facts.source_templates.get(&SpanKey::new(source_ref.span)),
            call_args_by_scope.get(&scope).map(Vec::as_slice),
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
) {
    for candidate in candidates {
        for resolved_path in resolve_helper_paths(source_path, &candidate) {
            let reads = summarize_helper(&resolved_path, summaries, active);
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
    RelativeSuffix(Vec<TemplatePart>),
}

#[derive(Debug, Clone)]
enum TemplatePart {
    Literal(String),
    Arg(usize),
}

fn collect_ast_facts(script: &Script, model: &SemanticModel, source: &str) -> AstFacts {
    let mut facts = AstFacts {
        source_templates: FxHashMap::default(),
        calls: Vec::new(),
    };
    walk_commands(&script.commands, model, source, &mut facts);
    facts
}

fn walk_commands(commands: &[Command], model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    for command in commands {
        walk_command(command, model, source, facts);
    }
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
                    && let Some(template) = source_path_template(argument, source)
                {
                    facts
                        .source_templates
                        .insert(SpanKey::new(command.span), template);
                }
            }

            walk_assignments(&command.assignments, model, source, facts);
            walk_word(&command.name, model, source, facts);
            walk_words(&command.args, model, source, facts);
            walk_redirects(&command.redirects, model, source, facts);
        }
        Command::Builtin(BuiltinCommand::Break(command)) => {
            walk_assignments(&command.assignments, model, source, facts);
            if let Some(word) = &command.depth {
                walk_word(word, model, source, facts);
            }
            walk_words(&command.extra_args, model, source, facts);
            walk_redirects(&command.redirects, model, source, facts);
        }
        Command::Builtin(BuiltinCommand::Continue(command)) => {
            walk_assignments(&command.assignments, model, source, facts);
            if let Some(word) = &command.depth {
                walk_word(word, model, source, facts);
            }
            walk_words(&command.extra_args, model, source, facts);
            walk_redirects(&command.redirects, model, source, facts);
        }
        Command::Builtin(BuiltinCommand::Return(command)) => {
            walk_assignments(&command.assignments, model, source, facts);
            if let Some(word) = &command.code {
                walk_word(word, model, source, facts);
            }
            walk_words(&command.extra_args, model, source, facts);
            walk_redirects(&command.redirects, model, source, facts);
        }
        Command::Builtin(BuiltinCommand::Exit(command)) => {
            walk_assignments(&command.assignments, model, source, facts);
            if let Some(word) = &command.code {
                walk_word(word, model, source, facts);
            }
            walk_words(&command.extra_args, model, source, facts);
            walk_redirects(&command.redirects, model, source, facts);
        }
        Command::Decl(command) => {
            walk_assignments(&command.assignments, model, source, facts);
            for operand in &command.operands {
                match operand {
                    DeclOperand::Flag(word) | DeclOperand::Dynamic(word) => {
                        walk_word(word, model, source, facts);
                    }
                    DeclOperand::Name(_) => {}
                    DeclOperand::Assignment(assignment) => {
                        walk_assignment(assignment, model, source, facts);
                    }
                }
            }
            walk_redirects(&command.redirects, model, source, facts);
        }
        Command::Pipeline(command) => walk_commands(&command.commands, model, source, facts),
        Command::List(command) => {
            walk_command(command.first.as_ref(), model, source, facts);
            for (_, command) in &command.rest {
                walk_command(command, model, source, facts);
            }
        }
        Command::Compound(command, redirects) => {
            walk_compound(command, model, source, facts);
            walk_redirects(redirects, model, source, facts);
        }
        Command::Function(FunctionDef { body, .. }) => walk_command(body, model, source, facts),
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
            walk_commands(&command.condition, model, source, facts);
            walk_commands(&command.then_branch, model, source, facts);
            for (condition, body) in &command.elif_branches {
                walk_commands(condition, model, source, facts);
                walk_commands(body, model, source, facts);
            }
            if let Some(body) = &command.else_branch {
                walk_commands(body, model, source, facts);
            }
        }
        CompoundCommand::For(command) => {
            if let Some(words) = &command.words {
                walk_words(words, model, source, facts);
            }
            walk_commands(&command.body, model, source, facts);
        }
        CompoundCommand::ArithmeticFor(command) => {
            walk_commands(&command.body, model, source, facts)
        }
        CompoundCommand::While(command) => {
            walk_commands(&command.condition, model, source, facts);
            walk_commands(&command.body, model, source, facts);
        }
        CompoundCommand::Until(command) => {
            walk_commands(&command.condition, model, source, facts);
            walk_commands(&command.body, model, source, facts);
        }
        CompoundCommand::Case(command) => {
            walk_word(&command.word, model, source, facts);
            for case in &command.cases {
                walk_words(&case.patterns, model, source, facts);
                walk_commands(&case.commands, model, source, facts);
            }
        }
        CompoundCommand::Select(command) => {
            walk_words(&command.words, model, source, facts);
            walk_commands(&command.body, model, source, facts);
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            walk_commands(commands, model, source, facts);
        }
        CompoundCommand::Arithmetic(_) => {}
        CompoundCommand::Time(command) => {
            if let Some(command) = &command.command {
                walk_command(command, model, source, facts);
            }
        }
        CompoundCommand::Conditional(command) => {
            walk_conditional_expr(&command.expression, model, source, facts)
        }
        CompoundCommand::Coproc(command) => walk_command(&command.body, model, source, facts),
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
    match &assignment.value {
        AssignmentValue::Scalar(word) => walk_word(word, model, source, facts),
        AssignmentValue::Array(words) => walk_words(words, model, source, facts),
    }
}

fn walk_words(words: &[Word], model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    for word in words {
        walk_word(word, model, source, facts);
    }
}

fn walk_word(word: &Word, model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    for part in &word.parts {
        match part {
            WordPart::CommandSubstitution(commands)
            | WordPart::ProcessSubstitution { commands, .. } => {
                walk_commands(commands, model, source, facts)
            }
            WordPart::ArithmeticExpansion(text) => walk_arithmetic(text, model, source, facts),
            WordPart::ParameterExpansion { operand, .. } => {
                if let Some(operand) = operand {
                    walk_source_text(operand, model, source, facts);
                }
            }
            WordPart::Substring { offset, length, .. }
            | WordPart::ArraySlice { offset, length, .. } => {
                walk_source_text(offset, model, source, facts);
                if let Some(length) = length {
                    walk_source_text(length, model, source, facts);
                }
            }
            WordPart::ArrayAccess { index, .. } => walk_source_text(index, model, source, facts),
            WordPart::IndirectExpansion { operand, .. } => {
                if let Some(operand) = operand {
                    walk_source_text(operand, model, source, facts);
                }
            }
            WordPart::Transformation { .. }
            | WordPart::Literal(_)
            | WordPart::Variable(_)
            | WordPart::Length(_)
            | WordPart::ArrayLength(_)
            | WordPart::ArrayIndices(_)
            | WordPart::PrefixMatch(_) => {}
        }
    }
}

fn walk_arithmetic(text: &SourceText, model: &SemanticModel, source: &str, facts: &mut AstFacts) {
    let inner = text.slice(source);
    if let Ok(output) = Parser::new(inner).parse() {
        walk_commands(&output.script.commands, model, inner, facts);
    }
}

fn walk_source_text(
    _text: &SourceText,
    _model: &SemanticModel,
    _source: &str,
    _facts: &mut AstFacts,
) {
}

fn walk_redirects(
    redirects: &[Redirect],
    model: &SemanticModel,
    source: &str,
    facts: &mut AstFacts,
) {
    for redirect in redirects {
        walk_word(&redirect.target, model, source, facts);
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
        ConditionalExpr::Word(word)
        | ConditionalExpr::Pattern(word)
        | ConditionalExpr::Regex(word) => walk_word(word, model, source, facts),
    }
}

fn source_path_template(word: &Word, source: &str) -> Option<SourcePathTemplate> {
    if static_word_text(word, source).is_some() {
        return None;
    }

    let mut parts = Vec::new();
    let mut ignored_root = false;
    let mut saw_dynamic = false;

    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) => {
                let text = text.as_str(source, span);
                if !text.is_empty() {
                    push_literal(&mut parts, text.to_owned());
                }
            }
            WordPart::Variable(name) => {
                if let Some(index) = positional_index(name) {
                    saw_dynamic = true;
                    parts.push(TemplatePart::Arg(index));
                } else if !ignored_root && parts.is_empty() {
                    ignored_root = true;
                    saw_dynamic = true;
                } else {
                    return None;
                }
            }
            _ => return None,
        }
    }

    (saw_dynamic && !parts.is_empty()).then_some(SourcePathTemplate::RelativeSuffix(parts))
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

fn source_candidates(
    kind: &SourceRefKind,
    template: Option<&SourcePathTemplate>,
    call_args: Option<&[Vec<Option<String>>]>,
) -> Vec<String> {
    match kind {
        SourceRefKind::DirectiveDevNull => Vec::new(),
        SourceRefKind::Literal(path) | SourceRefKind::Directive(path) => vec![path.clone()],
        SourceRefKind::Dynamic | SourceRefKind::SingleVariableStaticTail { .. } => {
            source_candidates_from_template(template, call_args)
        }
    }
}

fn source_candidates_from_template(
    template: Option<&SourcePathTemplate>,
    call_args: Option<&[Vec<Option<String>>]>,
) -> Vec<String> {
    let Some(template) = template else {
        return Vec::new();
    };

    match template {
        SourcePathTemplate::RelativeSuffix(parts) => {
            if uses_positional_args(parts) {
                call_args
                    .into_iter()
                    .flatten()
                    .filter_map(|args| render_relative_suffix(parts, args))
                    .collect()
            } else {
                render_relative_suffix(parts, &[]).into_iter().collect()
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

fn render_relative_suffix(parts: &[TemplatePart], args: &[Option<String>]) -> Option<String> {
    let mut rendered = String::new();
    for part in parts {
        match part {
            TemplatePart::Literal(text) => rendered.push_str(text),
            TemplatePart::Arg(index) => {
                let value = args.get(index.saturating_sub(1))?.as_ref()?;
                rendered.push_str(value);
            }
        }
    }

    let normalized = rendered
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

fn resolve_helper_paths(source_path: &Path, candidate: &str) -> Vec<PathBuf> {
    let candidate_path = Path::new(candidate);
    if candidate_path.is_absolute() {
        return candidate_path
            .exists()
            .then_some(candidate_path.to_path_buf())
            .into_iter()
            .collect();
    }

    let Some(base_dir) = source_path.parent() else {
        return Vec::new();
    };

    let direct = base_dir.join(candidate_path);
    if direct.exists() {
        return vec![direct];
    }

    let normalized = normalize_relative_candidate(candidate);
    if normalized.is_empty() {
        return Vec::new();
    }

    let flattened = normalized.replace('/', "__");
    let suffix = format!("__{flattened}");
    let Ok(entries) = fs::read_dir(base_dir) else {
        return Vec::new();
    };

    let mut matches = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name == flattened || name.ends_with(&suffix))
        })
        .collect::<Vec<_>>();
    matches.sort();
    matches.dedup();
    matches
}

fn normalize_relative_candidate(candidate: &str) -> String {
    let mut value = candidate.trim();
    while let Some(stripped) = value.strip_prefix("./") {
        value = stripped;
    }
    while let Some(stripped) = value.strip_prefix('/') {
        value = stripped;
    }
    value.to_owned()
}

fn summarize_helper(
    path: &Path,
    summaries: &mut FxHashMap<PathBuf, FxHashSet<Name>>,
    active: &mut FxHashSet<PathBuf>,
) -> FxHashSet<Name> {
    let key = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(summary) = summaries.get(&key) {
        return summary.clone();
    }
    if !active.insert(key.clone()) {
        return FxHashSet::default();
    }

    let summary = summarize_helper_uncached(&key, summaries, active);
    active.remove(&key);
    summaries.insert(key, summary.clone());
    summary
}

fn summarize_helper_uncached(
    path: &Path,
    summaries: &mut FxHashMap<PathBuf, FxHashSet<Name>>,
    active: &mut FxHashSet<PathBuf>,
) -> FxHashSet<Name> {
    let Ok(source) = fs::read_to_string(path) else {
        return FxHashSet::default();
    };
    let Ok(output) = Parser::new(&source).parse() else {
        return FxHashSet::default();
    };
    let indexer = Indexer::new(&source, &output);
    let semantic = SemanticModel::build(&output.script, &source, &indexer);

    let mut reads = semantic
        .unresolved_references()
        .iter()
        .map(|reference| semantic.reference(*reference).name.clone())
        .collect::<FxHashSet<_>>();
    reads.extend(
        collect_source_closure_reads_with_cache(
            &semantic,
            &output.script,
            &source,
            path,
            summaries,
            active,
        )
        .into_iter()
        .map(|read| read.name),
    );
    reads
}

fn static_word_text(word: &Word, source: &str) -> Option<String> {
    let mut result = String::new();
    for (part, span) in word.parts_with_spans() {
        match part {
            WordPart::Literal(text) => result.push_str(text.as_str(source, span)),
            _ => return None,
        }
    }
    Some(result)
}
