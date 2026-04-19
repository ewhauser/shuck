use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::{Command, DeclOperand, Span, Word};
use shuck_semantic::{BindingId, BindingKind, Reference, ReferenceKind};

use crate::facts::CommandId;
use crate::{Checker, ExpansionContext, WordFactContext};

pub(crate) struct ShellQuotingReuseAnalysis {
    pub assignment_spans: Vec<Span>,
    pub use_spans: Vec<Span>,
}

pub(crate) fn analyze_shell_quoting_reuse(checker: &Checker<'_>) -> ShellQuotingReuseAnalysis {
    let references = checker.semantic().references();
    let mut reference_indices = references
        .iter()
        .enumerate()
        .filter(|(_, reference)| {
            !matches!(
                reference.kind,
                ReferenceKind::DeclarationName | ReferenceKind::ImplicitRead
            )
        })
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    reference_indices.sort_unstable_by_key(|&index| references[index].span.start.offset);

    let scalar_bindings = checker
        .semantic()
        .bindings()
        .iter()
        .filter_map(|binding| {
            let context = binding_assignment_context(binding.kind)?;
            let word = checker.facts().binding_value(binding.id)?.scalar_word()?;
            Some(ScalarBinding {
                id: binding.id,
                word,
                context,
            })
        })
        .collect::<Vec<_>>();
    let scalar_binding_map = scalar_bindings
        .iter()
        .copied()
        .map(|binding| (binding.id, binding))
        .collect::<FxHashMap<_, _>>();

    let direct_unsafe_bindings = scalar_bindings
        .iter()
        .filter_map(|binding| {
            let fact = checker
                .facts()
                .word_fact(binding.word.span, binding.context)?;
            fact.contains_shell_quoting_literals().then_some(binding.id)
        })
        .collect::<FxHashSet<_>>();
    if direct_unsafe_bindings.is_empty() {
        return ShellQuotingReuseAnalysis {
            assignment_spans: Vec::new(),
            use_spans: Vec::new(),
        };
    }

    let dependency_map = scalar_bindings
        .iter()
        .map(|binding| {
            (
                binding.id,
                plain_scalar_reference_bindings(
                    binding.word.span,
                    checker,
                    references,
                    &reference_indices,
                ),
            )
        })
        .collect::<FxHashMap<_, _>>();

    let mut root_cache = FxHashMap::<BindingId, FxHashSet<BindingId>>::default();
    let mut used_root_bindings = FxHashSet::default();
    let mut use_spans = Vec::new();
    for fact in checker.facts().word_facts() {
        let Some(context) = fact.expansion_context() else {
            continue;
        };
        if !matches_sc2090_context(context) {
            continue;
        }
        if context != ExpansionContext::CommandName && command_is_eval(checker, fact.command_id()) {
            continue;
        }

        for span in fact.unquoted_scalar_expansion_spans().iter().copied() {
            let roots = root_bindings_for_expansion_span(
                span,
                checker,
                references,
                &reference_indices,
                &direct_unsafe_bindings,
                &dependency_map,
                &mut root_cache,
            );
            if roots.is_empty() {
                continue;
            }

            used_root_bindings.extend(roots);
            use_spans.push(span);
        }
    }

    use_spans.extend(export_name_spans(
        checker,
        &direct_unsafe_bindings,
        &dependency_map,
        &mut root_cache,
        &mut used_root_bindings,
    ));
    used_root_bindings.extend(export_assignment_root_bindings(
        checker,
        references,
        &reference_indices,
        &direct_unsafe_bindings,
        &dependency_map,
        &mut root_cache,
    ));

    sort_and_dedup_spans(&mut use_spans);

    let mut assignment_spans = used_root_bindings
        .iter()
        .filter_map(|binding_id| scalar_binding_map.get(binding_id).copied())
        .map(|binding| assignment_value_report_span(binding, checker))
        .collect::<Vec<_>>();
    sort_and_dedup_spans(&mut assignment_spans);

    ShellQuotingReuseAnalysis {
        assignment_spans,
        use_spans,
    }
}

#[derive(Clone, Copy)]
struct ScalarBinding<'a> {
    id: BindingId,
    word: &'a Word,
    context: WordFactContext,
}

fn binding_assignment_context(kind: BindingKind) -> Option<WordFactContext> {
    match kind {
        BindingKind::Assignment | BindingKind::AppendAssignment => Some(
            WordFactContext::Expansion(ExpansionContext::AssignmentValue),
        ),
        BindingKind::Declaration(_) => Some(WordFactContext::Expansion(
            ExpansionContext::DeclarationAssignmentValue,
        )),
        BindingKind::ParameterDefaultAssignment
        | BindingKind::ArrayAssignment
        | BindingKind::FunctionDefinition
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment
        | BindingKind::Nameref
        | BindingKind::Imported => None,
    }
}

fn matches_sc2090_context(context: ExpansionContext) -> bool {
    matches!(
        context,
        ExpansionContext::CommandName
            | ExpansionContext::CommandArgument
            | ExpansionContext::RedirectTarget(_)
            | ExpansionContext::HereString
    )
}

fn command_is_eval(checker: &Checker<'_>, command_id: CommandId) -> bool {
    checker
        .facts()
        .command(command_id)
        .effective_or_literal_name()
        == Some("eval")
}

fn plain_scalar_reference_bindings(
    word_span: Span,
    checker: &Checker<'_>,
    references: &[Reference],
    reference_indices: &[usize],
) -> Vec<BindingId> {
    let Some(fact) = checker
        .facts()
        .word_facts()
        .iter()
        .find(|fact| fact.span() == word_span)
    else {
        return Vec::new();
    };

    let bindings = fact
        .scalar_expansion_spans()
        .iter()
        .copied()
        .flat_map(|span| {
            direct_reference_bindings_in_span(span, checker, references, reference_indices, true)
        })
        .collect::<Vec<_>>();
    dedup_binding_ids(bindings)
}

fn root_bindings_for_expansion_span(
    expansion_span: Span,
    checker: &Checker<'_>,
    references: &[Reference],
    reference_indices: &[usize],
    direct_unsafe_bindings: &FxHashSet<BindingId>,
    dependency_map: &FxHashMap<BindingId, Vec<BindingId>>,
    root_cache: &mut FxHashMap<BindingId, FxHashSet<BindingId>>,
) -> FxHashSet<BindingId> {
    let mut roots = FxHashSet::default();
    for binding_id in direct_reference_bindings_in_span(
        expansion_span,
        checker,
        references,
        reference_indices,
        false,
    ) {
        roots.extend(root_bindings_for_binding(
            binding_id,
            direct_unsafe_bindings,
            dependency_map,
            root_cache,
            &mut FxHashSet::default(),
        ));
    }
    roots
}

fn root_bindings_for_binding(
    binding_id: BindingId,
    direct_unsafe_bindings: &FxHashSet<BindingId>,
    dependency_map: &FxHashMap<BindingId, Vec<BindingId>>,
    root_cache: &mut FxHashMap<BindingId, FxHashSet<BindingId>>,
    visiting: &mut FxHashSet<BindingId>,
) -> FxHashSet<BindingId> {
    if let Some(cached) = root_cache.get(&binding_id) {
        return cached.clone();
    }
    if !visiting.insert(binding_id) {
        return FxHashSet::default();
    }

    let mut roots = FxHashSet::default();
    if direct_unsafe_bindings.contains(&binding_id) {
        roots.insert(binding_id);
    }
    if let Some(dependencies) = dependency_map.get(&binding_id) {
        for dependency in dependencies {
            roots.extend(root_bindings_for_binding(
                *dependency,
                direct_unsafe_bindings,
                dependency_map,
                root_cache,
                visiting,
            ));
        }
    }

    visiting.remove(&binding_id);
    root_cache.insert(binding_id, roots.clone());
    roots
}

fn direct_reference_bindings_in_span(
    expansion_span: Span,
    checker: &Checker<'_>,
    references: &[Reference],
    reference_indices: &[usize],
    require_plain_reference: bool,
) -> Vec<BindingId> {
    let first_reference = reference_indices.partition_point(|&index| {
        references[index].span.start.offset < expansion_span.start.offset
    });

    let mut bindings = Vec::new();
    for &index in &reference_indices[first_reference..] {
        let reference = &references[index];
        if reference.span.start.offset > expansion_span.end.offset {
            break;
        }
        if !contains_span(expansion_span, reference.span)
            || (require_plain_reference
                && !expansion_span_is_plain_reference(expansion_span, reference, checker.source()))
        {
            continue;
        }
        if let Some(binding) = checker.semantic().resolved_binding(reference.id) {
            bindings.push(binding.id);
        }
    }
    dedup_binding_ids(bindings)
}

fn export_name_spans(
    checker: &Checker<'_>,
    direct_unsafe_bindings: &FxHashSet<BindingId>,
    dependency_map: &FxHashMap<BindingId, Vec<BindingId>>,
    root_cache: &mut FxHashMap<BindingId, FxHashSet<BindingId>>,
    used_root_bindings: &mut FxHashSet<BindingId>,
) -> Vec<Span> {
    checker
        .facts()
        .commands()
        .iter()
        .flat_map(|command| {
            let Command::Decl(clause) = command.command() else {
                return Vec::new();
            };
            if clause.variant.as_str() != "export" {
                return Vec::new();
            }

            clause
                .operands
                .iter()
                .filter_map(|operand| {
                    let DeclOperand::Name(reference) = operand else {
                        return None;
                    };
                    let binding = checker
                        .semantic()
                        .visible_binding(&reference.name, reference.span)?;
                    let roots = root_bindings_for_binding(
                        binding.id,
                        direct_unsafe_bindings,
                        dependency_map,
                        root_cache,
                        &mut FxHashSet::default(),
                    );
                    if roots.is_empty() {
                        return None;
                    }

                    used_root_bindings.extend(roots);
                    Some(reference.span)
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn export_assignment_root_bindings(
    checker: &Checker<'_>,
    references: &[Reference],
    reference_indices: &[usize],
    direct_unsafe_bindings: &FxHashSet<BindingId>,
    dependency_map: &FxHashMap<BindingId, Vec<BindingId>>,
    root_cache: &mut FxHashMap<BindingId, FxHashSet<BindingId>>,
) -> FxHashSet<BindingId> {
    let repeated_targets = repeated_export_assignment_targets(checker);
    if repeated_targets.is_empty() {
        return FxHashSet::default();
    }

    let mut roots = FxHashSet::default();
    for command in checker.facts().commands() {
        let Command::Decl(clause) = command.command() else {
            continue;
        };
        if clause.variant.as_str() != "export" {
            continue;
        }

        for operand in &clause.operands {
            let DeclOperand::Assignment(assignment) = operand else {
                continue;
            };
            if !repeated_targets.contains(assignment.target.name.as_str()) {
                continue;
            }
            let shuck_ast::AssignmentValue::Scalar(word) = &assignment.value else {
                continue;
            };

            for binding_id in
                plain_scalar_reference_bindings(word.span, checker, references, reference_indices)
            {
                roots.extend(root_bindings_for_binding(
                    binding_id,
                    direct_unsafe_bindings,
                    dependency_map,
                    root_cache,
                    &mut FxHashSet::default(),
                ));
            }
        }
    }

    roots
}

fn repeated_export_assignment_targets(checker: &Checker<'_>) -> FxHashSet<String> {
    let mut counts = FxHashMap::<String, usize>::default();
    for command in checker.facts().commands() {
        let Command::Decl(clause) = command.command() else {
            continue;
        };
        if clause.variant.as_str() != "export" {
            continue;
        }

        for operand in &clause.operands {
            let DeclOperand::Assignment(assignment) = operand else {
                continue;
            };
            *counts
                .entry(assignment.target.name.as_str().to_owned())
                .or_default() += 1;
        }
    }

    counts
        .into_iter()
        .filter_map(|(name, count)| (count > 1).then_some(name))
        .collect()
}

fn assignment_value_report_span(binding: ScalarBinding<'_>, checker: &Checker<'_>) -> Span {
    let source = checker.source();
    let word = binding.word;
    let Some(span) = source_shell_quoting_literal_run_span(word, source) else {
        return word.span;
    };
    span
}

fn source_shell_quoting_literal_run_span(word: &Word, source: &str) -> Option<Span> {
    let text = word.span.slice(source);
    let mut cursor = if word.is_fully_double_quoted() && text.starts_with('"') {
        1
    } else {
        0
    };
    let limit = if word.is_fully_double_quoted() && text.ends_with('"') {
        text.len().saturating_sub(1)
    } else {
        text.len()
    };
    let mut saw_expansion = false;
    let mut in_single = false;
    let mut in_double = word.is_fully_double_quoted() && text.starts_with('"');
    let mut index = cursor;

    while index < limit {
        let tail = &text[index..limit];
        let Some(ch) = tail.chars().next() else {
            break;
        };
        if ch == '\'' && !in_double && !text_position_is_escaped(text, index) {
            in_single = !in_single;
            index += ch.len_utf8();
            continue;
        }
        if ch == '"' && !in_single && !text_position_is_escaped(text, index) {
            in_double = !in_double;
            index += ch.len_utf8();
            continue;
        }
        if !in_single && matches!(ch, '$' | '`') && !text_position_is_escaped(text, index) {
            saw_expansion = true;
            if let Some(span) = shell_quoting_segment_span(word, text, cursor, index) {
                return Some(span);
            }
            index += expansion_len(tail);
            cursor = index;
            continue;
        }
        index += ch.len_utf8();
    }

    if let Some(span) = shell_quoting_segment_span(word, text, cursor, limit) {
        return Some(span);
    }
    if !saw_expansion && source_text_contains_shell_quoting_literals(&text[..limit]) {
        return Some(word.span);
    }

    None
}

fn expansion_len(text: &str) -> usize {
    if text.starts_with('`') {
        return closing_backtick_offset(text).unwrap_or(1);
    }
    if !text.starts_with('$') {
        return 1;
    }

    if text.starts_with("${") {
        return braced_expansion_len(text).unwrap_or(2);
    }
    if text.starts_with("$(") {
        return paren_expansion_len(text).unwrap_or(2);
    }

    let bytes = text.as_bytes();
    let Some(&next) = bytes.get(1) else {
        return 1;
    };
    if (next as char).is_ascii_alphabetic() || next == b'_' {
        let mut end = 2usize;
        while let Some(byte) = bytes.get(end) {
            let ch = *byte as char;
            if ch.is_ascii_alphanumeric() || ch == '_' {
                end += 1;
                continue;
            }
            break;
        }
        return end;
    }
    if (next as char).is_ascii_digit() || b"@*#?$!-".contains(&next) {
        return 2;
    }

    1
}

fn closing_backtick_offset(text: &str) -> Option<usize> {
    let mut chars = text.char_indices();
    chars.next()?;
    for (offset, ch) in chars {
        if ch == '`' && !text_position_is_escaped(text, offset) {
            return Some(offset + 1);
        }
    }

    None
}

fn braced_expansion_len(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in text.char_indices() {
        match ch {
            '$' if offset == 0 => {}
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(offset + 1);
                }
            }
            _ => {}
        }
    }

    None
}

fn paren_expansion_len(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (offset, ch) in text.char_indices() {
        match ch {
            '$' if offset == 0 => {}
            '(' => depth += 1,
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(offset + 1);
                }
            }
            _ => {}
        }
    }

    None
}

fn text_position_is_escaped(text: &str, offset: usize) -> bool {
    let bytes = text.as_bytes();
    let mut cursor = offset;
    let mut backslashes = 0usize;
    while cursor > 0 {
        cursor -= 1;
        if bytes[cursor] != b'\\' {
            break;
        }
        backslashes += 1;
    }

    backslashes % 2 == 1
}

fn source_text_contains_shell_quoting_literals(text: &str) -> bool {
    if text.contains(['"', '\'']) {
        return true;
    }

    let chars = text.chars().collect::<Vec<_>>();
    let mut index = 0usize;
    while index < chars.len() {
        if chars[index] != '\\' {
            index += 1;
            continue;
        }

        let mut end = index + 1;
        while end < chars.len() && chars[end] == '\\' {
            end += 1;
        }
        if chars.get(end).is_some_and(|next| {
            matches!(next, '"' | '\'') || (next.is_whitespace() && !matches!(next, '\n' | '\r'))
        }) {
            return true;
        }

        index = end;
    }

    false
}

fn shell_quoting_segment_span(word: &Word, text: &str, start: usize, end: usize) -> Option<Span> {
    let segment = &text[start..end];
    if !source_text_contains_shell_quoting_literals(segment) {
        return None;
    }

    let trimmed_start = if let Some(anchor) = first_escape_anchor(segment) {
        segment[..anchor]
            .rfind('\'')
            .map_or(start, |quote| start + quote + 1)
    } else {
        start
    };

    Some(Span::from_positions(
        word.span.start.advanced_by(&text[..trimmed_start]),
        word.span.start.advanced_by(&text[..end]),
    ))
}

fn first_shell_quoting_anchor(text: &str) -> Option<usize> {
    let chars = text.char_indices().collect::<Vec<_>>();
    for (index, (offset, ch)) in chars.iter().copied().enumerate() {
        if matches!(ch, '"' | '\'') {
            return Some(offset);
        }
        if ch != '\\' {
            continue;
        }
        if let Some((_, next)) = chars.get(index + 1).copied()
            && (matches!(next, '"' | '\'') || next.is_whitespace())
        {
            return Some(offset);
        }
    }

    None
}

fn first_escape_anchor(text: &str) -> Option<usize> {
    let chars = text.char_indices().collect::<Vec<_>>();
    for (index, (offset, ch)) in chars.iter().copied().enumerate() {
        if ch != '\\' {
            continue;
        }
        if let Some((_, next)) = chars.get(index + 1).copied()
            && (matches!(next, '"' | '\'') || next.is_whitespace())
        {
            return Some(offset);
        }
    }

    first_shell_quoting_anchor(text)
}

fn expansion_span_is_plain_reference(
    expansion_span: Span,
    reference: &Reference,
    source: &str,
) -> bool {
    let text = expansion_span.slice(source);
    text == format!("${}", reference.name.as_str())
        || text == format!("${{{}}}", reference.name.as_str())
}

fn contains_span(outer: Span, inner: Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn sort_and_dedup_spans(spans: &mut Vec<Span>) {
    spans.sort_unstable_by_key(|span| (span.start.offset, span.end.offset));
    spans.dedup();
}

fn dedup_binding_ids(bindings: Vec<BindingId>) -> Vec<BindingId> {
    let mut seen = FxHashSet::default();
    bindings
        .into_iter()
        .filter(|binding_id| seen.insert(*binding_id))
        .collect()
}
