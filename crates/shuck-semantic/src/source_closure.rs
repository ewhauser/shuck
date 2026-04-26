use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};

use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Name, Span};
use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;
use shuck_parser::{ShellDialect as ParseShellDialect, ShellProfile, ZshOptionState};

use crate::{
    Binding, BindingId, BindingKind, ContractCertainty, FileContract, FunctionContract,
    FunctionScopeKind, ProvidedBinding, ProvidedBindingKind, ScopeId, ScopeKind, SemanticModel,
    SourcePathResolver, SourceRefDiagnosticClass, SourceRefKind, SourceRefResolution, SpanKey,
    SyntheticRead, build_semantic_model_base_arena, infer_explicit_parse_dialect_from_source,
};

#[derive(Debug, Clone)]
struct SourceClosureContracts {
    synthetic_reads: Vec<SyntheticRead>,
    imported_bindings: Vec<ImportedBindingContractSite>,
    imported_functions: Vec<ImportedFunctionContractSite>,
    source_ref_resolutions: Vec<SourceRefResolution>,
    source_ref_explicitness: Vec<bool>,
    source_ref_diagnostic_classes: Vec<SourceRefDiagnosticClass>,
}

type SourceClosureContractResult = (
    Vec<SyntheticRead>,
    Vec<ImportedBindingContractSite>,
    Vec<SourceRefResolution>,
    Vec<bool>,
    Vec<SourceRefDiagnosticClass>,
);

type SourceRefMetadataResult = (
    Vec<SourceRefResolution>,
    Vec<bool>,
    Vec<SourceRefDiagnosticClass>,
);

#[derive(Clone)]
struct SourceClosureLookupContext<'a> {
    source_path_resolver: Option<&'a (dyn SourcePathResolver + Send + Sync)>,
    analyzed_paths: Option<&'a FxHashSet<PathBuf>>,
    shell_profile: ShellProfile,
    resolved_helper_paths: RefCell<FxHashMap<HelperPathResolutionKey, Vec<PathBuf>>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HelperPathResolutionKey {
    source_path: PathBuf,
    candidate: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct HelperSummaryKey {
    path: PathBuf,
    shell_profile: ShellProfileKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ShellProfileKey {
    dialect: ParseShellDialect,
    options: Option<ZshOptionState>,
}

impl ShellProfileKey {
    fn from_profile(profile: &ShellProfile) -> Self {
        Self {
            dialect: profile.dialect,
            options: profile.options.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ImportedBindingContractSite {
    pub(crate) scope: ScopeId,
    pub(crate) span: Span,
    pub(crate) binding: ProvidedBinding,
    pub(crate) origin_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct ImportedFunctionContractSite {
    scope: ScopeId,
    span: Span,
    certainty: ContractCertainty,
    trust_provided_bindings: bool,
    contract: FunctionContract,
}

pub(crate) fn collect_source_closure_contracts_from_model(
    model: &SemanticModel,
    source_path: &Path,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
    analyzed_paths: Option<&FxHashSet<PathBuf>>,
) -> SourceClosureContractResult {
    let mut summaries = FxHashMap::default();
    let mut active = FxHashSet::default();
    let context = SourceClosureLookupContext {
        source_path_resolver,
        analyzed_paths,
        shell_profile: model.shell_profile().clone(),
        resolved_helper_paths: RefCell::new(FxHashMap::default()),
    };
    let contracts = collect_source_closure_contracts_with_cache(
        model,
        source_path,
        &mut summaries,
        &mut active,
        &context,
    );
    (
        contracts.synthetic_reads,
        contracts.imported_bindings,
        contracts.source_ref_resolutions,
        contracts.source_ref_explicitness,
        contracts.source_ref_diagnostic_classes,
    )
}

pub(crate) fn collect_source_ref_metadata(
    model: &SemanticModel,
    source_path: &Path,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
    analyzed_paths: Option<&FxHashSet<PathBuf>>,
) -> SourceRefMetadataResult {
    let facts = collect_ast_facts(model);
    let call_args_by_scope = if facts.source_templates_use_positional_args {
        resolve_literal_call_args_by_scope(model, &facts.calls)
    } else {
        FxHashMap::default()
    };
    let context = SourceClosureLookupContext {
        source_path_resolver,
        analyzed_paths,
        shell_profile: model.shell_profile().clone(),
        resolved_helper_paths: RefCell::new(FxHashMap::default()),
    };
    let mut source_ref_resolutions = Vec::new();
    let mut source_ref_explicitness = Vec::new();
    let mut source_ref_diagnostic_classes = Vec::new();

    for source_ref in model.source_refs() {
        let scope = model.scope_at(source_ref.span.start.offset);
        let template = facts.source_templates.get(&SpanKey::new(source_ref.span));
        let candidates = source_candidates(
            &source_ref.kind,
            template,
            call_args_by_scope.get(&scope).map(Vec::as_slice),
            source_path,
        );
        let (resolved, explicit) =
            source_ref_metadata_for_candidates(source_path, candidates, &context);

        source_ref_resolutions.push(classify_source_ref_resolution(&source_ref.kind, resolved));
        source_ref_explicitness.push(explicit);
        source_ref_diagnostic_classes
            .push(classify_source_ref_diagnostic_class(source_ref, template));
    }

    (
        source_ref_resolutions,
        source_ref_explicitness,
        source_ref_diagnostic_classes,
    )
}

fn collect_source_closure_contracts_with_cache(
    model: &SemanticModel,
    source_path: &Path,
    summaries: &mut FxHashMap<HelperSummaryKey, FileContract>,
    active: &mut FxHashSet<HelperSummaryKey>,
    context: &SourceClosureLookupContext<'_>,
) -> SourceClosureContracts {
    let facts = collect_ast_facts(model);
    let call_args_by_scope = if facts.source_templates_use_positional_args {
        resolve_literal_call_args_by_scope(model, &facts.calls)
    } else {
        FxHashMap::default()
    };
    let mut synthetic_reads = Vec::new();
    let mut imported_bindings = Vec::new();
    let mut imported_functions = Vec::new();
    let mut source_ref_resolutions = Vec::new();
    let mut source_ref_explicitness = Vec::new();
    let mut source_ref_diagnostic_classes = Vec::new();

    for source_ref in model.source_refs() {
        let scope = model.scope_at(source_ref.span.start.offset);
        let template = facts.source_templates.get(&SpanKey::new(source_ref.span));
        let candidates = source_candidates(
            &source_ref.kind,
            template,
            call_args_by_scope.get(&scope).map(Vec::as_slice),
            source_path,
        );

        let (contract, resolved, explicit) =
            merge_contracts_for_candidates(source_path, candidates, summaries, active, context);
        let trust_provided_bindings =
            source_ref_can_import_provided_bindings(&source_ref.kind, template);
        source_ref_resolutions.push(classify_source_ref_resolution(&source_ref.kind, resolved));
        source_ref_explicitness.push(explicit);
        source_ref_diagnostic_classes
            .push(classify_source_ref_diagnostic_class(source_ref, template));
        if trust_provided_bindings {
            for provided in contract.provided_bindings.iter().cloned() {
                imported_bindings.push(ImportedBindingContractSite {
                    scope,
                    span: source_ref.span,
                    origin_paths: binding_origin_paths(&contract, &provided),
                    binding: provided,
                });
            }
        }
        imported_functions.extend(imported_function_sites_for_contract(
            scope,
            source_ref.span,
            &contract,
            trust_provided_bindings,
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
            if function_site.trust_provided_bindings {
                for binding in &function_site.contract.provided_bindings {
                    imported_bindings.push(ImportedBindingContractSite {
                        scope: call.scope,
                        span: call.span,
                        binding: binding_for_imported_function_call(
                            binding,
                            function_site.certainty,
                        ),
                        origin_paths: Vec::new(),
                    });
                }
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
        source_ref_diagnostic_classes,
    }
}

fn merge_contracts_for_candidates(
    source_path: &Path,
    candidates: impl IntoIterator<Item = String>,
    summaries: &mut FxHashMap<HelperSummaryKey, FileContract>,
    active: &mut FxHashSet<HelperSummaryKey>,
    context: &SourceClosureLookupContext<'_>,
) -> (FileContract, bool, bool) {
    let mut contracts = Vec::new();
    let mut resolved = false;
    let mut explicit = false;
    for candidate in candidates {
        let resolved_paths = resolve_helper_paths_cached(source_path, &candidate, context);
        resolved |= !resolved_paths.is_empty();
        explicit |= resolved_paths
            .iter()
            .any(|path| path_is_explicitly_analyzed(path, context.analyzed_paths));
        for resolved_path in resolved_paths {
            contracts.push(summarize_helper(&resolved_path, summaries, active, context));
        }
    }
    (
        FileContract::merge_candidate_contracts(&contracts),
        resolved,
        explicit,
    )
}

fn source_ref_metadata_for_candidates(
    source_path: &Path,
    candidates: impl IntoIterator<Item = String>,
    context: &SourceClosureLookupContext<'_>,
) -> (bool, bool) {
    let mut resolved = false;
    let mut explicit = false;
    for candidate in candidates {
        let resolved_paths = resolve_helper_paths_cached(source_path, &candidate, context);
        resolved |= !resolved_paths.is_empty();
        explicit |= resolved_paths
            .iter()
            .any(|path| path_is_explicitly_analyzed(path, context.analyzed_paths));
    }

    (resolved, explicit)
}

fn path_is_explicitly_analyzed(path: &Path, analyzed_paths: Option<&FxHashSet<PathBuf>>) -> bool {
    analyzed_paths.is_some_and(|paths| {
        paths.contains(path)
            || paths.contains(&fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
    })
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

fn source_ref_can_import_provided_bindings(
    kind: &SourceRefKind,
    template: Option<&SourcePathTemplate>,
) -> bool {
    match kind {
        SourceRefKind::Literal(_) | SourceRefKind::Directive(_) => true,
        SourceRefKind::DirectiveDevNull => false,
        SourceRefKind::Dynamic | SourceRefKind::SingleVariableStaticTail { .. } => {
            template.is_some_and(template_has_current_source_anchor)
        }
    }
}

fn template_has_current_source_anchor(template: &SourcePathTemplate) -> bool {
    match template {
        SourcePathTemplate::Interpolated(parts) => parts
            .iter()
            .any(|part| matches!(part, TemplatePart::SourceDir | TemplatePart::SourceFile)),
    }
}

fn classify_source_ref_diagnostic_class(
    source_ref: &crate::SourceRef,
    template: Option<&SourcePathTemplate>,
) -> SourceRefDiagnosticClass {
    match source_ref.kind {
        SourceRefKind::Dynamic if template_is_untracked_file(template) => {
            SourceRefDiagnosticClass::UntrackedFile
        }
        _ => source_ref.diagnostic_class,
    }
}

fn template_is_untracked_file(template: Option<&SourcePathTemplate>) -> bool {
    let Some(SourcePathTemplate::Interpolated(parts)) = template else {
        return false;
    };

    matches!(
        parts.as_slice(),
        [TemplatePart::Literal(path)] if path.contains('/')
    ) || matches!(
        parts.as_slice(),
        [TemplatePart::SourceDir, TemplatePart::Literal(tail)] if tail.starts_with('/')
    )
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
    bindings: Vec<ImportedBindingContractSite>,
) -> Vec<ImportedBindingContractSite> {
    let mut merged = FxHashMap::default();
    for site in bindings {
        let ImportedBindingContractSite {
            scope,
            span,
            binding,
            origin_paths,
        } = site;
        let key = (scope, span.start.offset, binding.name.clone(), binding.kind);
        let entry = merged
            .entry(key)
            .or_insert((span, binding.certainty, Vec::<PathBuf>::new()));
        entry.1 = entry.1.merge_same_site(binding.certainty);
        merge_origin_paths(&mut entry.2, &origin_paths);
    }

    let mut deduped = Vec::new();
    for ((scope, _, name, kind), (span, certainty, origin_paths)) in merged {
        deduped.push(ImportedBindingContractSite {
            scope,
            span,
            binding: ProvidedBinding::new(name, kind, certainty),
            origin_paths,
        });
    }
    deduped
}

fn merge_origin_paths(dest: &mut Vec<PathBuf>, origins: &[PathBuf]) {
    for origin in origins {
        if !dest.contains(origin) {
            dest.push(origin.clone());
        }
    }
}

fn imported_function_sites_for_contract(
    scope: ScopeId,
    span: Span,
    contract: &FileContract,
    trust_provided_bindings: bool,
) -> Vec<ImportedFunctionContractSite> {
    contract
        .provided_functions
        .iter()
        .cloned()
        .map(|function| ImportedFunctionContractSite {
            scope,
            span,
            certainty: function_contract_certainty(contract, &function.name),
            trust_provided_bindings,
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

fn binding_origin_paths(contract: &FileContract, binding: &ProvidedBinding) -> Vec<PathBuf> {
    if binding.kind != ProvidedBindingKind::Function {
        return Vec::new();
    }

    contract
        .provided_functions
        .iter()
        .find(|function| function.name == binding.name)
        .map(|function| function.origin_paths.clone())
        .unwrap_or_default()
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
    source_templates_use_positional_args: bool,
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
        source_templates_use_positional_args: false,
        calls: Vec::new(),
    };
    let program = model.recorded_program();
    let mut commands = program.commands().iter().collect::<Vec<_>>();
    commands.sort_by_key(|command| (command.span.start.offset, command.span.end.offset));

    for command in commands {
        let Some(info) = program.command_infos.get(&SpanKey::new(command.span)) else {
            continue;
        };
        let Some(name) = info.static_callee.as_deref() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }

        facts.calls.push(CallInfo {
            name: Name::from(name),
            scope: model.scope_at(command.span.start.offset),
            span: command.span,
            args: info.static_args.to_vec(),
        });

        if matches!(name, "source" | ".")
            && let Some(template) = info.source_path_template.clone()
        {
            facts.source_templates_use_positional_args |=
                source_template_uses_positional_args(&template);
            facts
                .source_templates
                .insert(SpanKey::new(command.span), template);
        }
    }
    facts
}

fn source_template_uses_positional_args(template: &SourcePathTemplate) -> bool {
    match template {
        SourcePathTemplate::Interpolated(parts) => uses_positional_args(parts),
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

fn resolve_helper_paths_cached(
    source_path: &Path,
    candidate: &str,
    context: &SourceClosureLookupContext<'_>,
) -> Vec<PathBuf> {
    let key = HelperPathResolutionKey {
        source_path: source_path.to_path_buf(),
        candidate: candidate.to_owned(),
    };
    if let Some(paths) = context.resolved_helper_paths.borrow().get(&key) {
        return paths.clone();
    }

    let paths = resolve_helper_paths(source_path, candidate, context.source_path_resolver);
    context
        .resolved_helper_paths
        .borrow_mut()
        .insert(key, paths.clone());
    paths
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
    summaries: &mut FxHashMap<HelperSummaryKey, FileContract>,
    active: &mut FxHashSet<HelperSummaryKey>,
    context: &SourceClosureLookupContext<'_>,
) -> FileContract {
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let requested_key = HelperSummaryKey {
        path: canonical_path.clone(),
        shell_profile: ShellProfileKey::from_profile(&context.shell_profile),
    };
    if let Some(summary) = summaries.get(&requested_key) {
        return summary.clone();
    }

    let Ok(source) = fs::read_to_string(&canonical_path) else {
        return FileContract::default();
    };
    let shell_profile = helper_shell_profile(&source, &canonical_path, &context.shell_profile);
    let key = HelperSummaryKey {
        path: canonical_path.clone(),
        shell_profile: ShellProfileKey::from_profile(&shell_profile),
    };
    if let Some(summary) = summaries.get(&key) {
        let summary = summary.clone();
        summaries.insert(requested_key, summary.clone());
        return summary;
    }
    if !active.insert(key.clone()) {
        return FileContract::default();
    }

    let summary = summarize_helper_uncached(
        &canonical_path,
        &source,
        shell_profile,
        summaries,
        active,
        context.source_path_resolver,
    );
    active.remove(&key);
    summaries.insert(key, summary.clone());
    summaries.insert(requested_key, summary.clone());
    summary
}

fn summarize_helper_uncached(
    path: &Path,
    source: &str,
    shell_profile: ShellProfile,
    summaries: &mut FxHashMap<HelperSummaryKey, FileContract>,
    active: &mut FxHashSet<HelperSummaryKey>,
    source_path_resolver: Option<&(dyn SourcePathResolver + Send + Sync)>,
) -> FileContract {
    let output = Parser::with_profile(source, shell_profile.clone()).parse();
    if output.is_err() {
        return FileContract::default();
    }
    let indexer = Indexer::new(source, &output);
    let mut observer = crate::NoopTraversalObserver;
    let mut semantic = build_semantic_model_base_arena(
        &output.arena_file,
        source,
        &indexer,
        &mut observer,
        Some(path),
        Some(shell_profile.clone()),
    );
    let collected = collect_source_closure_contracts_with_cache(
        &semantic,
        path,
        summaries,
        active,
        &SourceClosureLookupContext {
            source_path_resolver,
            analyzed_paths: None,
            shell_profile,
            resolved_helper_paths: RefCell::new(FxHashMap::default()),
        },
    );
    semantic.apply_source_contracts(
        collected.synthetic_reads.clone(),
        collected.imported_bindings.clone(),
        collected.source_ref_resolutions.clone(),
        collected.source_ref_explicitness.clone(),
        collected.source_ref_diagnostic_classes.clone(),
    );
    let analysis = semantic.analysis();

    let mut contract =
        summarize_scope_body_contract(&semantic, &analysis, ScopeId(0), &collected.synthetic_reads);
    let provided_functions = analysis.summarize_scope_provided_functions(ScopeId(0));
    for binding in &provided_functions {
        contract.add_provided_binding(binding.clone());
    }
    for function in build_scope_function_contracts(
        path,
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

fn helper_shell_profile(source: &str, path: &Path, inherited: &ShellProfile) -> ShellProfile {
    infer_explicit_parse_dialect_from_source(source, Some(path))
        .map(ShellProfile::native)
        .unwrap_or_else(|| inherited.clone())
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
    origin_path: &Path,
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
            function_contract.add_origin_path(origin_path.to_path_buf());
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use std::path::Path;

    use shuck_parser::parser::ShellDialect;
    #[cfg(windows)]
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn zsh_operation_operands_are_walked_when_collecting_ast_facts() {
        let source = "print ${(m)foo#$(printf '%s' \"$needle\")} ${(S)foo/$pattern/$(dirname \"$1\")} ${(m)foo:$(source \"$2\"):${length}}\n";
        let output = Parser::with_dialect(source, ShellDialect::Zsh)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build_arena(&output.arena_file, source, &indexer);
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

    #[test]
    fn wrapper_commands_keep_inner_call_and_source_template_facts() {
        let source = "\
#!/bin/bash
time . \"$1\"
coproc loader { . \"$2\"; }
";
        let output = Parser::with_dialect(source, ShellDialect::Bash)
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        let model = SemanticModel::build_arena(&output.arena_file, source, &indexer);
        let facts = collect_ast_facts(&model);
        let source_call_count = facts
            .calls
            .iter()
            .filter(|call| call.name.as_str() == ".")
            .count();

        assert_eq!(source_call_count, 2);
        assert_eq!(facts.source_templates.len(), 2);
    }

    #[test]
    fn summarize_helper_reuses_request_profile_cache_hit_before_reading() {
        let temp = tempdir().unwrap();
        let helper = temp.path().join("helper");
        std::fs::write(&helper, "#!/bin/zsh\nloaded_value=ok\n").unwrap();
        let canonical_helper = std::fs::canonicalize(&helper).unwrap();

        let context = SourceClosureLookupContext {
            source_path_resolver: None,
            analyzed_paths: None,
            shell_profile: ShellProfile::native(ParseShellDialect::Bash),
            resolved_helper_paths: RefCell::new(FxHashMap::default()),
        };
        let mut summaries = FxHashMap::default();
        let mut active = FxHashSet::default();

        let first = summarize_helper(&canonical_helper, &mut summaries, &mut active, &context);
        assert!(first.provided_bindings.iter().any(|binding| {
            binding.name.as_str() == "loaded_value"
                && binding.kind == ProvidedBindingKind::Variable
                && binding.certainty == ContractCertainty::Definite
        }));

        std::fs::remove_file(&helper).unwrap();

        let second = summarize_helper(&canonical_helper, &mut summaries, &mut active, &context);
        assert_eq!(second, first);
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
