use std::fs;
use std::path::{Path, PathBuf};

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{
    ArithmeticExpr, ArithmeticExprNode, BourneParameterExpansion, Command, File, Name,
    ParameterExpansion, ParameterExpansionSyntax, Span, StmtSeq, VarRef, Word, WordPart,
    WordPartNode,
};
use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;

use crate::{
    Binding, BindingId, BindingKind, ContractCertainty, FileContract, FunctionContract,
    FunctionScopeKind, ProvidedBinding, ProvidedBindingKind, ScopeId, ScopeKind, SemanticModel,
    SourcePathResolver, SourceRefKind, SourceRefResolution, SpanKey, SyntheticRead,
    build_semantic_model_base,
    cfg::{RecordedCommandId, RecordedCommandKind, RecordedCommandRange, RecordedProgram},
};

#[derive(Debug, Clone)]
struct SourceClosureContracts {
    synthetic_reads: Vec<SyntheticRead>,
    imported_bindings: Vec<(ScopeId, Span, ProvidedBinding)>,
    imported_functions: Vec<ImportedFunctionContractSite>,
    source_ref_resolutions: Vec<SourceRefResolution>,
    source_ref_explicitness: Vec<bool>,
}

type SourceClosureContractResult = (
    Vec<SyntheticRead>,
    Vec<(ScopeId, Span, ProvidedBinding)>,
    Vec<SourceRefResolution>,
    Vec<bool>,
);

#[derive(Clone, Copy)]
struct SourceClosureLookupContext<'a> {
    source_path_resolver: Option<&'a (dyn SourcePathResolver + Send + Sync)>,
    analyzed_paths: Option<&'a FxHashSet<PathBuf>>,
}

#[derive(Debug, Clone)]
struct ImportedFunctionContractSite {
    scope: ScopeId,
    span: Span,
    certainty: ContractCertainty,
    contract: FunctionContract,
}

pub(crate) fn collect_source_closure_contracts(
    model: &SemanticModel,
    file: &File,
    source: &str,
    source_path: &Path,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
    analyzed_paths: Option<&FxHashSet<PathBuf>>,
) -> SourceClosureContractResult {
    let mut summaries = FxHashMap::default();
    let mut active = FxHashSet::default();
    let context = SourceClosureLookupContext {
        source_path_resolver,
        analyzed_paths,
    };
    let contracts = collect_source_closure_contracts_with_cache(
        model,
        file,
        source,
        source_path,
        &mut summaries,
        &mut active,
        context,
    );
    (
        contracts.synthetic_reads,
        contracts.imported_bindings,
        contracts.source_ref_resolutions,
        contracts.source_ref_explicitness,
    )
}

fn collect_source_closure_contracts_with_cache(
    model: &SemanticModel,
    _file: &File,
    _source: &str,
    source_path: &Path,
    summaries: &mut FxHashMap<PathBuf, FileContract>,
    active: &mut FxHashSet<PathBuf>,
    context: SourceClosureLookupContext<'_>,
) -> SourceClosureContracts {
    let facts = collect_ast_facts(model);
    let call_args_by_scope = resolve_literal_call_args_by_scope(model, &facts.calls);
    let mut synthetic_reads = Vec::new();
    let mut imported_bindings = Vec::new();
    let mut imported_functions = Vec::new();
    let mut source_ref_resolutions = Vec::new();
    let mut source_ref_explicitness = Vec::new();

    for source_ref in model.source_refs() {
        let scope = model.scope_at(source_ref.span.start.offset);
        let candidates = source_candidates(
            &source_ref.kind,
            facts.source_templates.get(&SpanKey::new(source_ref.span)),
            call_args_by_scope.get(&scope).map(Vec::as_slice),
            source_path,
        );

        let (contract, resolved, explicit) =
            merge_contracts_for_candidates(source_path, candidates, summaries, active, context);
        source_ref_resolutions.push(classify_source_ref_resolution(&source_ref.kind, resolved));
        source_ref_explicitness.push(explicit);
        for provided in contract.provided_bindings.iter().cloned() {
            imported_bindings.push((scope, source_ref.span, provided));
        }
        imported_functions.extend(imported_function_sites_for_contract(
            scope,
            source_ref.span,
            &contract,
        ));
        for name in contract.required_reads {
            synthetic_reads.push(SyntheticRead {
                scope,
                span: source_ref.span,
                name,
            });
        }
    }

    for call in &facts.calls {
        if let Some(function_site) = visible_imported_function_contract(
            model,
            &imported_functions,
            &call.name,
            call.scope,
            call.span.start.offset,
        ) {
            for name in &function_site.contract.required_reads {
                synthetic_reads.push(SyntheticRead {
                    scope: call.scope,
                    span: call.span,
                    name: name.clone(),
                });
            }
            for binding in &function_site.contract.provided_bindings {
                imported_bindings.push((
                    call.scope,
                    call.span,
                    binding_for_imported_function_call(binding, function_site.certainty),
                ));
            }
        }

        let Some(candidate) = local_helper_command_candidate(&call.name) else {
            continue;
        };
        let (contract, _, _) =
            merge_contracts_for_candidates(source_path, [candidate], summaries, active, context);
        for name in contract.required_reads {
            synthetic_reads.push(SyntheticRead {
                scope: call.scope,
                span: call.span,
                name,
            });
        }
    }

    SourceClosureContracts {
        synthetic_reads: dedup_synthetic_reads(synthetic_reads),
        imported_bindings: dedup_imported_bindings(imported_bindings),
        imported_functions,
        source_ref_resolutions,
        source_ref_explicitness,
    }
}

fn merge_contracts_for_candidates(
    source_path: &Path,
    candidates: impl IntoIterator<Item = String>,
    summaries: &mut FxHashMap<PathBuf, FileContract>,
    active: &mut FxHashSet<PathBuf>,
    context: SourceClosureLookupContext<'_>,
) -> (FileContract, bool, bool) {
    let mut contracts = Vec::new();
    let mut resolved = false;
    let mut explicit = false;
    for candidate in candidates {
        let resolved_paths =
            resolve_helper_paths(source_path, &candidate, context.source_path_resolver);
        resolved |= !resolved_paths.is_empty();
        explicit |= resolved_paths.iter().any(|path| {
            context.analyzed_paths.is_some_and(|paths| {
                paths.contains(path)
                    || paths.contains(&fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
            })
        });
        for resolved_path in resolved_paths {
            contracts.push(summarize_helper(
                &resolved_path,
                summaries,
                active,
                context.source_path_resolver,
            ));
        }
    }
    (
        FileContract::merge_candidate_contracts(&contracts),
        resolved,
        explicit,
    )
}

fn classify_source_ref_resolution(kind: &SourceRefKind, resolved: bool) -> SourceRefResolution {
    match kind {
        SourceRefKind::DirectiveDevNull => SourceRefResolution::Resolved,
        SourceRefKind::Literal(_)
        | SourceRefKind::Directive(_)
        | SourceRefKind::Dynamic
        | SourceRefKind::SingleVariableStaticTail { .. } => {
            if resolved {
                SourceRefResolution::Resolved
            } else {
                SourceRefResolution::Unresolved
            }
        }
    }
}

fn dedup_synthetic_reads(reads: Vec<SyntheticRead>) -> Vec<SyntheticRead> {
    let mut seen = FxHashSet::default();
    let mut deduped = Vec::new();
    for read in reads {
        if seen.insert((read.scope, read.span.start.offset, read.name.clone())) {
            deduped.push(read);
        }
    }
    deduped
}

fn dedup_imported_bindings(
    bindings: Vec<(ScopeId, Span, ProvidedBinding)>,
) -> Vec<(ScopeId, Span, ProvidedBinding)> {
    let mut merged = FxHashMap::default();
    for (scope, span, binding) in bindings {
        let key = (scope, span.start.offset, binding.name.clone(), binding.kind);
        let entry = merged.entry(key).or_insert((span, binding.certainty));
        entry.1 = entry.1.merge_same_site(binding.certainty);
    }

    let mut deduped = Vec::new();
    for ((scope, _, name, kind), (span, certainty)) in merged {
        deduped.push((scope, span, ProvidedBinding::new(name, kind, certainty)));
    }
    deduped
}

fn imported_function_sites_for_contract(
    scope: ScopeId,
    span: Span,
    contract: &FileContract,
) -> Vec<ImportedFunctionContractSite> {
    contract
        .provided_functions
        .iter()
        .cloned()
        .map(|function| ImportedFunctionContractSite {
            scope,
            span,
            certainty: function_contract_certainty(contract, &function.name),
            contract: function,
        })
        .collect()
}

fn function_contract_certainty(contract: &FileContract, name: &Name) -> ContractCertainty {
    contract
        .provided_bindings
        .iter()
        .find(|binding| binding.kind == ProvidedBindingKind::Function && binding.name == *name)
        .map(|binding| binding.certainty)
        .unwrap_or(ContractCertainty::Definite)
}

fn binding_for_imported_function_call(
    binding: &ProvidedBinding,
    function_certainty: ContractCertainty,
) -> ProvidedBinding {
    let certainty = match (binding.certainty, function_certainty) {
        (ContractCertainty::Definite, ContractCertainty::Definite) => ContractCertainty::Definite,
        _ => ContractCertainty::Possible,
    };
    ProvidedBinding::new(binding.name.clone(), binding.kind, certainty)
}

enum VisibleFunctionTarget<'a> {
    Local,
    Imported(&'a ImportedFunctionContractSite),
}

fn visible_imported_function_contract<'a>(
    model: &SemanticModel,
    imported_functions: &'a [ImportedFunctionContractSite],
    name: &Name,
    scope: ScopeId,
    offset: usize,
) -> Option<&'a ImportedFunctionContractSite> {
    for scope_id in model.ancestor_scopes(scope) {
        let local = visible_local_function_binding_in_scope(model, name, scope_id, scope, offset)
            .map(|binding| {
                (
                    VisibleFunctionTarget::Local,
                    model.binding(binding).span.start.offset,
                )
            });
        let imported =
            visible_imported_function_in_scope(imported_functions, name, scope_id, scope, offset)
                .map(|site| {
                    (
                        VisibleFunctionTarget::Imported(site),
                        site.span.start.offset,
                    )
                });

        let visible = match (local, imported) {
            (Some((target, local_offset)), Some((imported_target, imported_offset))) => {
                if imported_offset > local_offset {
                    (imported_target, imported_offset)
                } else {
                    (target, local_offset)
                }
            }
            (Some(candidate), None) | (None, Some(candidate)) => candidate,
            (None, None) => continue,
        };

        return match visible.0 {
            VisibleFunctionTarget::Local => None,
            VisibleFunctionTarget::Imported(site) => Some(site),
        };
    }

    None
}

fn visible_local_function_binding_in_scope(
    model: &SemanticModel,
    name: &Name,
    target_scope: ScopeId,
    call_scope: ScopeId,
    offset: usize,
) -> Option<BindingId> {
    let candidates = model.scopes()[target_scope.index()].bindings.get(name)?;
    if target_scope != call_scope {
        return candidates.iter().rev().copied().find(|binding| {
            matches!(
                model.binding(*binding).kind,
                BindingKind::FunctionDefinition
            )
        });
    }

    candidates.iter().rev().copied().find(|binding| {
        let candidate = model.binding(*binding);
        matches!(candidate.kind, BindingKind::FunctionDefinition)
            && candidate.span.start.offset <= offset
    })
}

fn visible_imported_function_in_scope<'a>(
    imported_functions: &'a [ImportedFunctionContractSite],
    name: &Name,
    target_scope: ScopeId,
    call_scope: ScopeId,
    offset: usize,
) -> Option<&'a ImportedFunctionContractSite> {
    imported_functions
        .iter()
        .filter(|site| site.scope == target_scope && site.contract.name == *name)
        .filter(|site| target_scope != call_scope || site.span.start.offset <= offset)
        .max_by_key(|site| site.span.start.offset)
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
pub(crate) enum SourcePathTemplate {
    Interpolated(Vec<TemplatePart>),
}

#[derive(Debug, Clone)]
pub(crate) enum TemplatePart {
    Literal(String),
    Arg(usize),
    SourceDir,
    SourceFile,
}

fn collect_ast_facts(model: &SemanticModel) -> AstFacts {
    let mut facts = AstFacts {
        source_templates: FxHashMap::default(),
        calls: Vec::new(),
    };
    let program = model.recorded_program();
    collect_commands_in_range(model, program, program.file_commands(), &mut facts);
    facts
}

fn collect_commands_in_range(
    model: &SemanticModel,
    program: &RecordedProgram,
    range: RecordedCommandRange,
    facts: &mut AstFacts,
) {
    for &command in program.commands_in(range) {
        collect_command(model, program, command, facts);
    }
}

fn collect_command(
    model: &SemanticModel,
    program: &RecordedProgram,
    command_id: RecordedCommandId,
    facts: &mut AstFacts,
) {
    let command = program.command(command_id);
    if let Some(info) = program.command_infos.get(&SpanKey::new(command.span))
        && let Some(name) = info.static_callee.as_deref()
        && !name.is_empty()
    {
        facts.calls.push(CallInfo {
            name: Name::from(name),
            scope: model.scope_at(command.span.start.offset),
            span: command.span,
            args: info.static_args.iter().cloned().collect(),
        });

        if matches!(name, "source" | ".")
            && let Some(template) = info.source_path_template.clone()
        {
            facts
                .source_templates
                .insert(SpanKey::new(command.span), template);
        }
    }

    for region in program.nested_regions(command.nested_regions) {
        collect_commands_in_range(model, program, region.commands, facts);
    }

    match command.kind {
        RecordedCommandKind::Linear
        | RecordedCommandKind::Break { .. }
        | RecordedCommandKind::Continue { .. }
        | RecordedCommandKind::Return
        | RecordedCommandKind::Exit => {}
        RecordedCommandKind::List { first, rest } => {
            collect_command(model, program, first, facts);
            for item in program.list_items(rest) {
                collect_command(model, program, item.command, facts);
            }
        }
        RecordedCommandKind::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            collect_commands_in_range(model, program, condition, facts);
            collect_commands_in_range(model, program, then_branch, facts);
            for branch in program.elif_branches(elif_branches) {
                collect_commands_in_range(model, program, branch.condition, facts);
                collect_commands_in_range(model, program, branch.body, facts);
            }
            collect_commands_in_range(model, program, else_branch, facts);
        }
        RecordedCommandKind::While { condition, body }
        | RecordedCommandKind::Until { condition, body } => {
            collect_commands_in_range(model, program, condition, facts);
            collect_commands_in_range(model, program, body, facts);
        }
        RecordedCommandKind::For { body }
        | RecordedCommandKind::Select { body }
        | RecordedCommandKind::ArithmeticFor { body }
        | RecordedCommandKind::BraceGroup { body }
        | RecordedCommandKind::Subshell { body } => {
            collect_commands_in_range(model, program, body, facts);
        }
        RecordedCommandKind::Case { arms } => {
            for arm in program.case_arms(arms) {
                collect_commands_in_range(model, program, arm.commands, facts);
            }
        }
        RecordedCommandKind::Pipeline { segments } => {
            for segment in program.pipeline_segments(segments) {
                collect_command(model, program, segment.command, facts);
            }
        }
    }
}

pub(crate) fn source_path_template(
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
        if word_part_starts_with_shell_expansion(&part.kind)
            && span_is_backslash_escaped(source, &part.kind, part.span)
        {
            continue;
        }
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
                let value = path_to_template_string(source_path.parent()?);
                rendered.push_str(&value);
            }
            TemplatePart::SourceFile => {
                let value = path_to_template_string(source_path);
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

fn path_to_template_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
fn word_part_starts_with_shell_expansion(part: &WordPart) -> bool {
    matches!(
        part,
        WordPart::Variable(_)
            | WordPart::Parameter(_)
            | WordPart::ParameterExpansion { .. }
            | WordPart::CommandSubstitution { .. }
            | WordPart::ProcessSubstitution { .. }
            | WordPart::ArithmeticExpansion { .. }
    )
}

fn span_is_backslash_escaped(source: &str, part: &WordPart, span: Span) -> bool {
    let Some(offset) = expansion_start_offset(source, part, span) else {
        return false;
    };
    let bytes = source.as_bytes();
    if offset == 0 || offset > bytes.len() {
        return false;
    }

    let mut backslashes = 0usize;
    let mut index = offset;
    while index > 0 && bytes[index - 1] == b'\\' {
        backslashes += 1;
        index -= 1;
    }

    backslashes % 2 == 1
}

fn expansion_start_offset(source: &str, part: &WordPart, span: Span) -> Option<usize> {
    let bytes = source.as_bytes();
    let (line_start, line_end) = line_bounds(source, span.start.line)?;
    let search_start = line_start
        .saturating_add(span.start.column.saturating_sub(1))
        .min(line_end);

    (search_start..line_end).find(|&candidate| part_matches_expansion_start(bytes, candidate, part))
}

fn part_matches_expansion_start(bytes: &[u8], offset: usize, part: &WordPart) -> bool {
    match part {
        WordPart::Variable(_)
        | WordPart::Parameter(_)
        | WordPart::ParameterExpansion { .. }
        | WordPart::ArithmeticExpansion { .. } => bytes[offset] == b'$',
        WordPart::CommandSubstitution { .. } => matches!(bytes[offset], b'$' | b'`'),
        WordPart::ProcessSubstitution { .. } => {
            matches!(bytes[offset], b'<' | b'>') && bytes.get(offset + 1) == Some(&b'(')
        }
        _ => false,
    }
}

fn line_bounds(source: &str, line_number: usize) -> Option<(usize, usize)> {
    if line_number == 0 {
        return None;
    }

    let mut line = 1usize;
    let mut line_start = 0usize;
    for (index, byte) in source.bytes().enumerate() {
        if line == line_number {
            let line_end = source[index..]
                .find('\n')
                .map(|relative| index + relative)
                .unwrap_or(source.len());
            return Some((line_start, line_end));
        }

        if byte == b'\n' {
            line += 1;
            line_start = index + 1;
        }
    }

    (line == line_number).then_some((line_start, source.len()))
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
        if let ScopeKind::Function(FunctionScopeKind::Named(names)) = &scope.kind
            && let Some(parent) = scope.parent
        {
            for name in names {
                scopes_by_parent_and_name
                    .entry((parent, name.clone()))
                    .or_default()
                    .push(scope.id);
            }
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
    for candidate_path in candidate_path_variants(candidate) {
        if candidate_path.is_absolute() {
            if candidate_path.is_file() {
                return vec![candidate_path];
            }
            continue;
        }

        let Some(base_dir) = source_path.parent() else {
            return Vec::new();
        };

        let direct = base_dir.join(&candidate_path);
        if direct.is_file() {
            return vec![direct];
        }
    }

    source_path_resolver
        .into_iter()
        .flat_map(|resolver| resolver.resolve_candidate_paths(source_path, candidate))
        .filter(|path| path.is_file())
        .collect()
}

fn candidate_path_variants(candidate: &str) -> Vec<PathBuf> {
    #[cfg(not(windows))]
    let variants = vec![PathBuf::from(candidate)];
    #[cfg(windows)]
    let mut variants = vec![PathBuf::from(candidate)];
    #[cfg(windows)]
    if candidate.starts_with(r"\\?\") && candidate.contains('/') {
        // Windows canonicalize() can produce verbatim paths, which do not accept
        // forward slashes once we stitch in a Bash-style "/helper.bash" suffix.
        let normalized = PathBuf::from(candidate.replace('/', "\\"));
        if !variants.contains(&normalized) {
            variants.push(normalized);
        }
    }
    variants
}

fn summarize_helper(
    path: &Path,
    summaries: &mut FxHashMap<PathBuf, FileContract>,
    active: &mut FxHashSet<PathBuf>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> FileContract {
    let key = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Some(summary) = summaries.get(&key) {
        return summary.clone();
    }
    if !active.insert(key.clone()) {
        return FileContract::default();
    }

    let summary = summarize_helper_uncached(&key, summaries, active, source_path_resolver);
    active.remove(&key);
    summaries.insert(key, summary.clone());
    summary
}

fn summarize_helper_uncached(
    path: &Path,
    summaries: &mut FxHashMap<PathBuf, FileContract>,
    active: &mut FxHashSet<PathBuf>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> FileContract {
    let Ok(source) = fs::read_to_string(path) else {
        return FileContract::default();
    };
    let output = Parser::new(&source).parse();
    if output.is_err() {
        return FileContract::default();
    }
    let indexer = Indexer::new(&source, &output);
    let mut observer = crate::NoopTraversalObserver;
    let mut semantic = build_semantic_model_base(
        &output.file,
        &source,
        &indexer,
        &mut observer,
        Some(path),
        None,
    );
    let collected = collect_source_closure_contracts_with_cache(
        &semantic,
        &output.file,
        &source,
        path,
        summaries,
        active,
        SourceClosureLookupContext {
            source_path_resolver,
            analyzed_paths: None,
        },
    );
    semantic.apply_source_contracts(
        collected.synthetic_reads.clone(),
        collected.imported_bindings.clone(),
        collected.source_ref_resolutions.clone(),
        collected.source_ref_explicitness.clone(),
    );
    let analysis = semantic.analysis();

    let mut contract =
        summarize_scope_body_contract(&semantic, &analysis, ScopeId(0), &collected.synthetic_reads);
    let provided_functions = analysis.summarize_scope_provided_functions(ScopeId(0));
    for binding in &provided_functions {
        contract.add_provided_binding(binding.clone());
    }
    for function in build_scope_function_contracts(
        &semantic,
        &analysis,
        ScopeId(0),
        &collected.synthetic_reads,
        &collected.imported_functions,
        &provided_functions,
    ) {
        contract.add_provided_function(function);
    }
    contract
}

fn summarize_scope_body_contract(
    semantic: &SemanticModel,
    analysis: &crate::SemanticAnalysis<'_>,
    scope: ScopeId,
    synthetic_reads: &[SyntheticRead],
) -> FileContract {
    let scope_members = scope_members_excluding_functions(semantic.scopes(), scope);
    let mut contract = FileContract::default();
    for reference in semantic.unresolved_references() {
        let reference = semantic.reference(*reference);
        if scope_members.contains(&reference.scope) {
            contract.add_required_read(reference.name.clone());
        }
    }
    for read in synthetic_reads {
        if scope_members.contains(&read.scope) {
            contract.add_required_read(read.name.clone());
        }
    }
    for binding in analysis.summarize_scope_provided_bindings(scope) {
        contract.add_provided_binding(binding);
    }
    contract
}

fn build_scope_function_contracts(
    semantic: &SemanticModel,
    analysis: &crate::SemanticAnalysis<'_>,
    scope: ScopeId,
    synthetic_reads: &[SyntheticRead],
    imported_functions: &[ImportedFunctionContractSite],
    provided_functions: &[ProvidedBinding],
) -> Vec<FunctionContract> {
    let function_scopes = semantic
        .scopes()
        .iter()
        .filter_map(|candidate| {
            (candidate.parent == Some(scope))
                .then_some(candidate)
                .and_then(|candidate| match &candidate.kind {
                    ScopeKind::Function(FunctionScopeKind::Named(names)) => {
                        Some((candidate.id, names.clone()))
                    }
                    _ => None,
                })
        })
        .collect::<Vec<_>>();

    let mut local_contracts_by_scope = FxHashMap::default();
    let mut contracts_by_name: FxHashMap<Name, Vec<FunctionContract>> = FxHashMap::default();

    for (function_scope, names) in function_scopes {
        let body_contract = local_contracts_by_scope
            .entry(function_scope)
            .or_insert_with(|| {
                summarize_scope_body_contract(semantic, analysis, function_scope, synthetic_reads)
            })
            .clone();
        for name in names {
            let mut function_contract = FunctionContract::new(name.clone());
            for read in &body_contract.required_reads {
                function_contract.add_required_read(read.clone());
            }
            for binding in &body_contract.provided_bindings {
                function_contract.add_provided_binding(binding.clone());
            }
            contracts_by_name
                .entry(name)
                .or_default()
                .push(function_contract);
        }
    }

    for imported in imported_functions.iter().filter(|site| site.scope == scope) {
        contracts_by_name
            .entry(imported.contract.name.clone())
            .or_default()
            .push(imported.contract.clone());
    }

    let mut functions = Vec::new();
    for binding in provided_functions {
        if let Some(contracts) = contracts_by_name.get(&binding.name)
            && let Some(function) = FunctionContract::merge_candidate_contracts(contracts)
        {
            functions.push(function);
        }
    }
    functions.sort_by(|left, right| left.name.as_str().cmp(right.name.as_str()));
    functions
}

fn scope_members_excluding_functions(scopes: &[crate::Scope], root: ScopeId) -> FxHashSet<ScopeId> {
    let mut members = FxHashSet::default();
    let mut stack = vec![root];
    while let Some(scope_id) = stack.pop() {
        if !members.insert(scope_id) {
            continue;
        }
        for scope in scopes {
            if scope.parent == Some(scope_id) && !matches!(scope.kind, ScopeKind::Function(_)) {
                stack.push(scope.id);
            }
        }
    }
    members
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
    #[cfg(windows)]
    use std::path::Path;

    use shuck_parser::parser::ShellDialect;
    #[cfg(windows)]
    use std::fs;
    #[cfg(windows)]
    use tempfile::tempdir;

    #[test]
    fn zsh_operation_operands_are_walked_when_collecting_ast_facts() {
        let source = "print ${(m)foo#$(printf '%s' \"$needle\")} ${(S)foo/$pattern/$(dirname \"$1\")} ${(m)foo:$(source \"$2\"):${length}}\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build(&output.file, source, &indexer);
        let facts = collect_ast_facts(&model);
        let call_names = facts
            .calls
            .iter()
            .map(|call| call.name.to_string())
            .collect::<Vec<_>>();

        assert!(call_names.iter().any(|name| name == "printf"));
        assert!(call_names.iter().any(|name| name == "dirname"));
        assert!(call_names.iter().any(|name| name == "source"));
    }
    #[cfg(windows)]
    #[test]
    fn source_dir_templates_render_windows_paths_with_shell_separators() {
        let candidate = render_template_candidate(
            &[
                TemplatePart::SourceDir,
                TemplatePart::Literal("/helper.bash".to_owned()),
            ],
            &[],
            Path::new(r"C:\workspace\loader.bash"),
        );

        assert_eq!(candidate.as_deref(), Some("C:/workspace/helper.bash"));
    }

    #[cfg(windows)]
    #[test]
    fn resolve_helper_paths_accepts_verbatim_candidates_with_shell_separators() {
        let temp = tempdir().unwrap();
        let loader = temp.path().join("loader.bash");
        let helper = temp.path().join("helper.bash");
        fs::write(&loader, "#!/bin/bash\n").unwrap();
        fs::write(&helper, "#!/bin/bash\n").unwrap();

        let canonical_loader = fs::canonicalize(&loader).unwrap();
        let candidate = format!(
            "{}/helper.bash",
            canonical_loader.parent().unwrap().to_string_lossy()
        );

        let resolved = resolve_helper_paths(&canonical_loader, &candidate, None);

        assert_eq!(resolved.len(), 1);
        assert_eq!(
            fs::canonicalize(&resolved[0]).unwrap(),
            fs::canonicalize(&helper).unwrap()
        );
    }
}
