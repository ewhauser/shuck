use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use shuck_ast::{Name, Position, Span};

use crate::facts::ComparableNameUseKind;
use crate::{Checker, Rule, Violation};

use super::variable_reference_common::{
    VariableReferenceFilter, has_same_name_defining_bindings, is_environment_style_name,
    is_reportable_variable_reference,
};

pub struct PossibleVariableMisspelling {
    pub reference: String,
    pub candidate: String,
}

impl Violation for PossibleVariableMisspelling {
    fn rule() -> Rule {
        Rule::PossibleVariableMisspelling
    }

    fn message(&self) -> String {
        format!(
            "reference to `{}` looks like it may mean `{}`",
            self.reference, self.candidate
        )
    }
}

pub fn possible_variable_misspelling(checker: &mut Checker) {
    let suppressed_reference_spans = checker
        .semantic()
        .references()
        .iter()
        .filter(|reference| {
            checker
                .semantic()
                .is_guarded_parameter_reference(reference.id)
                || checker
                    .semantic()
                    .is_defaulting_parameter_operand_reference(reference.id)
        })
        .fold(FxHashMap::default(), |mut spans, reference| {
            spans
                .entry(reference.name.to_string())
                .or_insert_with(Vec::new)
                .push(reference.span);
            spans
        });
    let guarded_name_offsets = checker
        .semantic()
        .references()
        .iter()
        .filter(|reference| {
            checker
                .semantic()
                .is_guarded_parameter_reference(reference.id)
        })
        .fold(FxHashMap::default(), |mut offsets, reference| {
            offsets
                .entry(reference.name.to_string())
                .or_insert_with(Vec::new)
                .push(reference.span.start.offset);
            offsets
        });

    let mut candidate_cache = FxHashMap::<String, Option<String>>::default();
    let mut findings = Vec::new();
    for uninitialized in checker
        .semantic_analysis()
        .uninitialized_references()
        .iter()
    {
        let reference = checker.semantic().reference(uninitialized.reference);
        if !is_reportable_variable_reference(
            checker,
            reference,
            VariableReferenceFilter {
                suppress_environment_style_names: false,
            },
        ) {
            continue;
        }
        if !looks_like_case_mismatch_reference(reference.name.as_str()) {
            continue;
        }
        if is_known_runtime_name(reference.name.as_str()) {
            continue;
        }
        if is_internal_placeholder_name(reference.name.as_str()) {
            continue;
        }
        if has_prior_guarded_reference(
            &guarded_name_offsets,
            reference.name.as_str(),
            reference.span,
        ) {
            continue;
        }
        if has_same_name_defining_bindings(checker, &reference.name) {
            continue;
        }
        if is_presence_tested_reference_name(checker, reference.name.as_str(), reference.span) {
            continue;
        }
        if is_assignment_target_variant_reference(checker, reference.name.as_str(), reference.span)
        {
            continue;
        }
        if is_build_flag_alias_assignment_value(checker, reference.name.as_str(), reference.span) {
            continue;
        }

        let Some(candidate) =
            cached_candidate_name(&mut candidate_cache, checker, reference.name.as_str())
        else {
            continue;
        };
        if is_build_flag_family_non_report_pair(reference.name.as_str(), candidate.as_str()) {
            continue;
        }
        if is_hostid_label_echo(reference.name.as_str(), reference.span, checker.source()) {
            continue;
        }
        if is_parallel_c_and_cxx_flag_use(
            checker,
            reference.name.as_str(),
            reference.span,
            candidate.as_str(),
        ) {
            continue;
        }
        if is_literal_numbered_suffix_variant(
            checker.source(),
            reference.name.as_str(),
            reference.span,
            candidate.as_str(),
        ) {
            continue;
        }
        findings.push((reference.span, reference.name.to_string(), candidate));
    }
    findings.extend(heredoc_findings(
        checker,
        &guarded_name_offsets,
        &suppressed_reference_spans,
        &mut candidate_cache,
    ));
    if source_may_have_scope_compat_misspelling(checker.source()) {
        findings.extend(scope_compat_findings(
            checker,
            &guarded_name_offsets,
            &suppressed_reference_spans,
        ));
    }

    findings.sort_by_key(|(span, _, _)| (span.start.offset, span.end.offset));
    let mut reported_names = FxHashSet::default();

    for (span, reference, candidate) in findings {
        if !reported_names.insert(reference.clone()) {
            continue;
        }
        checker.report(
            PossibleVariableMisspelling {
                reference,
                candidate,
            },
            span,
        );
    }
}

fn heredoc_findings(
    checker: &Checker<'_>,
    guarded_name_offsets: &FxHashMap<String, Vec<usize>>,
    suppressed_reference_spans: &FxHashMap<String, Vec<Span>>,
    candidate_cache: &mut FxHashMap<String, Option<String>>,
) -> Vec<(Span, String, String)> {
    let mut findings = Vec::new();
    let mut seen = FxHashSet::default();

    for command in checker.facts().commands() {
        for name_use in command.scope_heredoc_name_read_uses() {
            if name_use.kind() != ComparableNameUseKind::Parameter {
                continue;
            }
            let reference_name = name_use.key().as_str();
            if !seen.insert((reference_name.to_owned(), name_use.span().start.offset)) {
                continue;
            }
            if !looks_like_case_mismatch_reference(reference_name) {
                continue;
            }
            if is_known_runtime_name(reference_name) || is_internal_placeholder_name(reference_name)
            {
                continue;
            }
            let candidate = match cached_candidate_name(candidate_cache, checker, reference_name) {
                Some(candidate) => candidate,
                None => continue,
            };
            if has_prior_guarded_reference(guarded_name_offsets, reference_name, name_use.span()) {
                continue;
            }
            if has_suppressed_reference_span(
                suppressed_reference_spans,
                reference_name,
                name_use.span(),
            ) {
                continue;
            }
            if has_same_name_defining_bindings(checker, &Name::from(reference_name)) {
                continue;
            }
            if is_presence_tested_reference_name(checker, reference_name, name_use.span()) {
                continue;
            }
            if is_build_flag_family_non_report_pair(reference_name, candidate.as_str()) {
                continue;
            }
            if is_hostid_label_echo(reference_name, name_use.span(), checker.source()) {
                continue;
            }
            if is_parallel_c_and_cxx_flag_use(
                checker,
                reference_name,
                name_use.span(),
                candidate.as_str(),
            ) {
                continue;
            }
            if is_literal_numbered_suffix_variant(
                checker.source(),
                reference_name,
                name_use.span(),
                candidate.as_str(),
            ) {
                continue;
            }

            findings.push((
                parameter_reference_span(checker.source(), name_use.span()),
                reference_name.to_owned(),
                candidate,
            ));
        }
    }

    findings
}

fn scope_compat_findings(
    checker: &Checker<'_>,
    guarded_name_offsets: &FxHashMap<String, Vec<usize>>,
    suppressed_reference_spans: &FxHashMap<String, Vec<Span>>,
) -> Vec<(Span, String, String)> {
    let mut findings = Vec::new();
    let mut seen = FxHashSet::default();
    let mut index = ScopeCompatIndex::default();
    add_scope_compat_binding_candidates(&mut index, checker);
    add_scope_compat_presence_test_candidates(&mut index, checker);
    add_scope_compat_name_uses(&mut index, checker);

    for name_use in index.references() {
        let reference_name = name_use.name.as_str();
        if !seen.insert((name_use.name.clone(), name_use.span.start.offset)) {
            continue;
        }
        if !looks_like_case_mismatch_reference(reference_name) {
            continue;
        }
        if is_known_runtime_name(reference_name) || is_internal_placeholder_name(reference_name) {
            continue;
        }
        if has_prior_guarded_reference(guarded_name_offsets, reference_name, name_use.span) {
            continue;
        }
        if has_suppressed_reference_span(suppressed_reference_spans, reference_name, name_use.span)
        {
            continue;
        }
        if has_same_name_defining_bindings(checker, &Name::from(reference_name)) {
            continue;
        }
        if is_presence_tested_reference_name(checker, reference_name, name_use.span) {
            continue;
        }

        let Some(candidate) = preferred_scope_compat_candidate_name_fast(&index, reference_name)
        else {
            continue;
        };
        if is_hostid_label_echo(reference_name, name_use.span, checker.source()) {
            continue;
        }
        if is_parallel_c_and_cxx_flag_use(
            checker,
            reference_name,
            name_use.span,
            candidate.as_str(),
        ) {
            continue;
        }
        if is_literal_numbered_suffix_variant(
            checker.source(),
            reference_name,
            name_use.span,
            candidate.as_str(),
        ) {
            continue;
        }

        findings.push((
            parameter_reference_span(checker.source(), name_use.span),
            reference_name.to_owned(),
            candidate,
        ));
    }

    findings
}

#[derive(Default)]
struct ScopeCompatIndex {
    exact_candidates: FxHashMap<String, ScopeCompatCandidate>,
    build_flag_candidates: FxHashMap<String, ScopeCompatCandidate>,
    shellspec_execdir_references: Vec<ScopeCompatUse>,
    build_flag_references: Vec<ScopeCompatUse>,
}

#[derive(Clone)]
struct ScopeCompatCandidate {
    name: String,
    span: Span,
}

struct ScopeCompatUse {
    name: String,
    span: Span,
}

impl ScopeCompatIndex {
    fn references(&self) -> impl Iterator<Item = &ScopeCompatUse> {
        self.shellspec_execdir_references
            .iter()
            .chain(self.build_flag_references.iter())
    }

    fn add_exact_candidate(&mut self, name: &str, span: Span) {
        insert_earliest_candidate(&mut self.exact_candidates, name.to_owned(), name, span);
    }

    fn add_build_flag_candidate(&mut self, name: &str, span: Span) {
        insert_earliest_candidate(&mut self.build_flag_candidates, name.to_owned(), name, span);
    }

    fn best_exact_candidate(&self, name: &str) -> Option<String> {
        self.exact_candidates
            .get(name)
            .map(|candidate| candidate.name.clone())
    }

    fn best_candidate_by_name(&self, name: &str) -> Option<&ScopeCompatCandidate> {
        self.build_flag_candidates.get(name)
    }
}

fn insert_earliest_candidate(
    candidates: &mut FxHashMap<String, ScopeCompatCandidate>,
    key: String,
    name: &str,
    span: Span,
) {
    let candidate = ScopeCompatCandidate {
        name: name.to_owned(),
        span,
    };
    candidates
        .entry(key)
        .and_modify(|current| {
            if (span.start.offset, span.end.offset)
                < (current.span.start.offset, current.span.end.offset)
            {
                *current = candidate.clone();
            }
        })
        .or_insert(candidate);
}

fn add_scope_compat_binding_candidates(index: &mut ScopeCompatIndex, checker: &Checker<'_>) {
    for binding in checker.semantic().bindings() {
        let name = binding.name.as_str();
        if name == "SHELLSPEC_SPECDIR" {
            index.add_exact_candidate(name, binding.span);
        }
        if is_reportable_build_flag_family_name(name) {
            index.add_build_flag_candidate(name, binding.span);
        }
    }
}

fn add_scope_compat_presence_test_candidates(index: &mut ScopeCompatIndex, checker: &Checker<'_>) {
    for (name, span) in checker
        .facts()
        .presence_test_candidate_spans(checker.semantic())
    {
        let name = name.as_str();
        if name == "SHELLSPEC_SPECDIR" {
            index.add_exact_candidate(name, span);
        }
        if is_reportable_build_flag_family_name(name) {
            index.add_build_flag_candidate(name, span);
        }
    }
}

fn add_scope_compat_name_uses(index: &mut ScopeCompatIndex, checker: &Checker<'_>) {
    for name_use in checker
        .facts()
        .possible_variable_misspelling_scope_compat_name_uses()
    {
        let name = name_use.key().as_str();
        if name == "SHELLSPEC_EXECDIR" {
            index.shellspec_execdir_references.push(ScopeCompatUse {
                name: name.to_owned(),
                span: name_use.span(),
            });
            continue;
        }
        if name == "SHELLSPEC_SPECDIR" {
            index.add_exact_candidate(name, name_use.span());
            continue;
        }
        if !is_reportable_build_flag_family_name(name) {
            continue;
        }

        index.add_build_flag_candidate(name, name_use.span());
        if name_use.kind() != ComparableNameUseKind::Derived
            || !is_braced_parameter_use(checker.source(), name_use.span())
        {
            continue;
        }
        index.build_flag_references.push(ScopeCompatUse {
            name: name.to_owned(),
            span: name_use.span(),
        });
    }
}

fn preferred_scope_compat_candidate_name_fast(
    index: &ScopeCompatIndex,
    reference_name: &str,
) -> Option<String> {
    if reference_name == "SHELLSPEC_EXECDIR" {
        return index.best_exact_candidate("SHELLSPEC_SPECDIR");
    }
    best_build_flag_candidate(index, reference_name)
}

fn best_build_flag_candidate(index: &ScopeCompatIndex, reference_name: &str) -> Option<String> {
    let (prefix, reference_suffix) = split_build_flag_family_name(reference_name)?;
    compatible_build_flag_candidate_suffixes(reference_suffix)
        .iter()
        .filter_map(|candidate_suffix| {
            let candidate_name = if prefix.is_empty() {
                (*candidate_suffix).to_owned()
            } else {
                format!("{prefix}{candidate_suffix}")
            };
            index.best_candidate_by_name(&candidate_name)
        })
        .min_by_key(|candidate| (candidate.span.start.offset, candidate.span.end.offset))
        .map(|candidate| candidate.name.clone())
}

fn compatible_build_flag_candidate_suffixes(reference_suffix: &str) -> &'static [&'static str] {
    match reference_suffix {
        "CFLAGS" => &["CXXFLAGS", "CPPFLAGS"],
        "CPPFLAGS" => &["CXXFLAGS"],
        "CXXFLAGS" => &["CPPFLAGS"],
        _ => &[],
    }
}

fn source_may_have_scope_compat_misspelling(source: &str) -> bool {
    source.contains("SHELLSPEC_EXECDIR")
        || source.contains("CFLAGS")
        || source.contains("CPPFLAGS")
        || source.contains("CXXFLAGS")
}

fn is_braced_parameter_use(source: &str, span: Span) -> bool {
    source
        .as_bytes()
        .get(span.start.offset..span.start.offset + 2)
        .is_some_and(|prefix| prefix == b"${")
}

fn has_prior_guarded_reference(
    guarded_name_offsets: &FxHashMap<String, Vec<usize>>,
    name: &str,
    span: Span,
) -> bool {
    guarded_name_offsets
        .get(name)
        .is_some_and(|offsets| offsets.iter().any(|offset| *offset < span.start.offset))
}

fn has_suppressed_reference_span(
    suppressed_reference_spans: &FxHashMap<String, Vec<Span>>,
    name: &str,
    span: Span,
) -> bool {
    suppressed_reference_spans.get(name).is_some_and(|spans| {
        spans
            .iter()
            .copied()
            .any(|other| spans_overlap(other, span))
    })
}

fn spans_overlap(left: Span, right: Span) -> bool {
    left.start.offset < right.end.offset && right.start.offset < left.end.offset
}

fn is_presence_tested_reference_name(
    checker: &Checker<'_>,
    reference_name: &str,
    reference_span: Span,
) -> bool {
    checker
        .facts()
        .is_presence_tested_name(&Name::from(reference_name), reference_span)
}

fn parameter_reference_span(source: &str, span: Span) -> Span {
    let Some(previous_offset) = span.start.offset.checked_sub(1) else {
        return span;
    };
    if source.as_bytes().get(previous_offset) != Some(&b'$') {
        return span;
    }
    let Some(start) = position_at_offset(source, previous_offset) else {
        return span;
    };
    Span::from_positions(start, span.end)
}

fn position_at_offset(source: &str, target_offset: usize) -> Option<Position> {
    if target_offset > source.len() {
        return None;
    }
    let mut position = Position::new();
    for char in source[..target_offset].chars() {
        position.advance(char);
    }
    Some(position)
}

fn looks_like_case_mismatch_reference(name: &str) -> bool {
    is_environment_style_name(name)
        && name.len() >= 3
        && name.chars().any(|char| char.is_ascii_uppercase())
}

fn preferred_candidate_name(checker: &Checker<'_>, target_name: &str) -> Option<String> {
    checker
        .facts()
        .possible_variable_misspelling_candidate(checker.semantic(), target_name)
}

fn cached_candidate_name(
    cache: &mut FxHashMap<String, Option<String>>,
    checker: &Checker<'_>,
    target_name: &str,
) -> Option<String> {
    cache
        .entry(target_name.to_owned())
        .or_insert_with(|| preferred_candidate_name(checker, target_name))
        .clone()
}

fn canonical_uppercase_name(name: &str) -> String {
    name.chars().map(|char| char.to_ascii_uppercase()).collect()
}

fn is_assignment_target_variant_reference(
    checker: &Checker<'_>,
    reference_name: &str,
    reference_span: shuck_ast::Span,
) -> bool {
    let Some(target_name) = checker
        .facts()
        .assignment_value_target_name_for_span(reference_span)
        .map(|name| name.as_str())
    else {
        return false;
    };

    reference_name
        .strip_prefix(target_name)
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.chars().all(|char| char.is_ascii_digit())
        })
}

fn is_build_flag_alias_assignment_value(
    checker: &Checker<'_>,
    reference_name: &str,
    reference_span: shuck_ast::Span,
) -> bool {
    if reference_name != "LDFLAGS" {
        return false;
    }

    checker
        .facts()
        .assignment_value_target_name_for_span(reference_span)
        .map(|name| name.as_str())
        .is_some_and(|target_name| {
            target_name
                .strip_suffix(reference_name)
                .is_some_and(|prefix| matches!(prefix, "MY" | "GO" | "CGO_" | "EXTRA_"))
        })
}

fn is_build_flag_family_non_report_pair(reference_name: &str, candidate_name: &str) -> bool {
    if !is_environment_style_name(candidate_name) {
        return false;
    }

    let candidate_upper = canonical_uppercase_name(candidate_name);
    if !is_build_flag_family_name(reference_name) || !is_build_flag_family_name(&candidate_upper) {
        return false;
    }

    !is_build_flag_misspelling_report_pair(reference_name, &candidate_upper)
}

fn is_build_flag_misspelling_report_pair(reference_name: &str, candidate_name: &str) -> bool {
    let candidate_upper = canonical_uppercase_name(candidate_name);
    let Some((reference_prefix, reference_suffix)) = split_build_flag_family_name(reference_name)
    else {
        return false;
    };
    let Some((candidate_prefix, candidate_suffix)) = split_build_flag_family_name(&candidate_upper)
    else {
        return false;
    };
    if reference_prefix != candidate_prefix {
        return false;
    }

    matches!(
        (reference_suffix, candidate_suffix),
        ("CFLAGS", "CXXFLAGS" | "CPPFLAGS") | ("CPPFLAGS", "CXXFLAGS") | ("CXXFLAGS", "CPPFLAGS")
    )
}

fn split_build_flag_family_name(name: &str) -> Option<(&str, &'static str)> {
    ["CXXFLAGS", "CPPFLAGS", "CFLAGS", "LDFLAGS", "GOFLAGS"]
        .into_iter()
        .find_map(|suffix| {
            if name == suffix {
                Some(("", suffix))
            } else {
                name.strip_suffix(suffix)
                    .filter(|prefix| prefix.ends_with('_'))
                    .map(|prefix| (prefix, suffix))
            }
        })
}

fn is_reportable_build_flag_family_name(name: &str) -> bool {
    let Some((_, suffix)) = split_build_flag_family_name(name) else {
        return false;
    };
    matches!(suffix, "CFLAGS" | "CPPFLAGS" | "CXXFLAGS")
}

fn is_build_flag_family_name(name: &str) -> bool {
    matches!(
        name,
        "CFLAGS" | "CXXFLAGS" | "CPPFLAGS" | "LDFLAGS" | "GOFLAGS"
    ) || name.ends_with("_CFLAGS")
        || name.ends_with("_CXXFLAGS")
        || name.ends_with("_CPPFLAGS")
        || name.ends_with("_LDFLAGS")
}

fn is_parallel_c_and_cxx_flag_use(
    checker: &Checker<'_>,
    reference_name: &str,
    reference_span: shuck_ast::Span,
    candidate_name: &str,
) -> bool {
    if reference_name != "CPPFLAGS" || canonical_uppercase_name(candidate_name) != "CXXFLAGS" {
        return false;
    }

    let source = checker.source();
    let current_line = source_line_at(source, reference_span.start.offset);
    if text_mentions_shell_name(current_line, "CFLAGS") {
        return true;
    }

    let nearby_lines = source_line_window(source, reference_span.start.offset, 1);
    text_mentions_shell_name(nearby_lines, "CPPFLAGS")
        && text_mentions_shell_name(nearby_lines, "CFLAGS")
        && text_mentions_shell_name(nearby_lines, "CXXFLAGS")
}

fn is_hostid_label_echo(
    reference_name: &str,
    reference_span: shuck_ast::Span,
    source: &str,
) -> bool {
    if reference_name != "HOSTID" {
        return false;
    }

    let line_start = source[..reference_span.start.offset]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let line = source_line_at(source, reference_span.start.offset);
    let reference_column = reference_span.start.offset.saturating_sub(line_start);
    let Some(prefix) = line.get(..reference_column) else {
        return false;
    };

    is_echo_hostid_label_prefix(prefix)
}

fn is_echo_hostid_label_prefix(prefix: &str) -> bool {
    let Some((command, mut rest)) = split_first_shell_token(prefix.trim_start()) else {
        return false;
    };
    if command != "echo" && command != "${ECHOCMD}" && command != "$ECHOCMD" {
        return false;
    }

    rest = rest.trim_start();
    while let Some(after_dash) = rest.strip_prefix('-') {
        let Some((flag, after_flag)) = after_dash.split_once(char::is_whitespace) else {
            return false;
        };
        if flag.is_empty() || !flag.chars().all(|char| matches!(char, 'e' | 'E' | 'n')) {
            return false;
        }
        rest = after_flag.trim_start();
    }

    rest.trim_start_matches(['"', '\'']) == "hostid="
}

fn split_first_shell_token(text: &str) -> Option<(&str, &str)> {
    let split_at = text.find(char::is_whitespace)?;
    Some((&text[..split_at], &text[split_at..]))
}

fn is_literal_numbered_suffix_variant(
    source: &str,
    reference_name: &str,
    reference_span: Span,
    candidate_name: &str,
) -> bool {
    let candidate_upper = canonical_uppercase_name(candidate_name);
    let Some(suffix) = candidate_upper.strip_prefix(reference_name) else {
        return false;
    };
    if suffix.is_empty() || !suffix.chars().all(|char| char.is_ascii_digit()) {
        return false;
    }

    source_suffix_matches(source, reference_span.end.offset, suffix)
        || source
            .as_bytes()
            .get(reference_span.end.offset)
            .is_some_and(|byte| *byte == b'}')
            && source_suffix_matches(source, reference_span.end.offset + 1, suffix)
}

fn source_suffix_matches(source: &str, offset: usize, suffix: &str) -> bool {
    source
        .as_bytes()
        .get(offset..offset + suffix.len())
        .is_some_and(|source_suffix| source_suffix.eq_ignore_ascii_case(suffix.as_bytes()))
}

fn is_known_runtime_name(name: &str) -> bool {
    matches!(
        name,
        "IFS"
            | "USER"
            | "HOME"
            | "SHELL"
            | "PWD"
            | "TERM"
            | "PATH"
            | "CDPATH"
            | "LANG"
            | "SUDO_USER"
            | "DOAS_USER"
            | "PPID"
            | "HOSTNAME"
            | "SECONDS"
            | "LINENO"
            | "FUNCNAME"
            | "BASH_SOURCE"
            | "BASH_LINENO"
            | "RANDOM"
            | "PIPESTATUS"
            | "BASH_REMATCH"
            | "READLINE_LINE"
            | "BASH_VERSION"
            | "BASH_VERSINFO"
            | "OSTYPE"
            | "HISTCONTROL"
            | "HISTSIZE"
            | "EUID"
            | "TMPDIR"
            | "GEM_HOME"
            | "GEM_PATH"
    ) || name.starts_with("LC_")
}

fn is_internal_placeholder_name(name: &str) -> bool {
    name.strip_prefix("_SHUCK_GHA_")
        .is_some_and(|suffix| suffix.chars().all(|char| char.is_ascii_digit()))
}

fn source_line_at(source: &str, offset: usize) -> &str {
    let start = line_start_offset(source, offset);
    let end = source[offset..]
        .find('\n')
        .map_or(source.len(), |index| offset + index);
    &source[start..end]
}

fn line_start_offset(source: &str, offset: usize) -> usize {
    source[..offset].rfind('\n').map_or(0, |index| index + 1)
}

fn source_line_window(source: &str, offset: usize, radius: usize) -> &str {
    let mut start = offset;
    for _ in 0..=radius {
        start = source[..start].rfind('\n').map_or(0, |index| index);
        if start == 0 {
            break;
        }
    }

    let mut end = offset;
    for _ in 0..=radius {
        end = source[end..]
            .find('\n')
            .map_or(source.len(), |index| end + index + 1);
        if end == source.len() {
            break;
        }
    }

    &source[start..end]
}

fn text_mentions_shell_name(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(start, _)| {
        let end = start + name.len();
        let before = start
            .checked_sub(1)
            .and_then(|index| text.as_bytes().get(index))
            .copied();
        let after = text.as_bytes().get(end).copied();

        before.is_none_or(|byte| !is_shell_name_byte(byte))
            && after.is_none_or(|byte| !is_shell_name_byte(byte))
    })
}

fn is_shell_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_exact_uppercase_fold_matches() {
        let source = "\
#!/bin/sh
package_name=demo
echo \"$PACKAGE_NAME\"
FooBar=demo
echo \"$FOOBAR\"
foo1=demo
echo \"$FOO1\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$PACKAGE_NAME", "$FOOBAR", "$FOO1"]
        );
    }

    #[test]
    fn prefers_exact_case_fold_candidate_over_edit_distance_candidate() {
        let source = "\
#!/bin/sh
package_name=demo
PACKAG_NAME=demo
echo \"$PACKAGE_NAME\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("`package_name`"));
    }

    #[test]
    fn prefers_binding_candidates_over_presence_test_candidates() {
        let source = "\
#!/bin/sh
PACKAG_NAME=demo
if [ -n \"$package_name\" ]; then :; fi
echo \"$PACKAGE_NAME\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("`PACKAG_NAME`"));
    }

    #[test]
    fn keeps_binding_tie_breaking_by_first_span() {
        let source = "\
#!/bin/sh
ALPHA_VALUF=demo
ALPHA_VALUE=demo
echo \"$ALPHA_VALUG\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("`ALPHA_VALUF`"));
    }

    #[test]
    fn rejects_weak_two_edit_shapes() {
        let source = "\
#!/bin/sh
ABCDEF=demo
echo \"$ABXYEF\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_only_the_first_occurrence_of_a_name() {
        let source = "\
#!/bin/sh
package_name=demo
echo \"$PACKAGE_NAME\"
echo \"$PACKAGE_NAME\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$PACKAGE_NAME"]
        );
    }

    #[test]
    fn ignores_short_names_mixed_case_refs_and_underscore_removal() {
        let source = "\
#!/bin/sh
bar=demo
echo \"$BAR\"
foo=demo
echo \"$Foo\"
echo \"$fOo\"
foo_bar=demo
echo \"$FOOBAR\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_common_environment_style_typos() {
        let source = "\
#!/bin/sh
PRGNAM=demo
PROFILE=core
LIBDIRSUFFIX=64
SLKCFLAGS='-O2'
echo \"$PKGNAM\"
echo \"$PROFILES\"
echo \"$LIBDIRSUFIX\"
echo \"$SLKFLAGS\"
echo \"$SLCKFLAGS\"
echo \"$SLKCFLAG\"
echo \"$BRANCH_\"
BRANCH=stable
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$PKGNAM",
                "$PROFILES",
                "$LIBDIRSUFIX",
                "$SLKFLAGS",
                "$SLCKFLAGS",
                "$SLKCFLAG",
                "$BRANCH_"
            ]
        );
    }

    #[test]
    fn reports_environment_style_alias_families() {
        let source = "\
#!/bin/sh
foo_bar=demo
PKG_CONFIG=pkg-config
XBPS_REMOVE_CMD=rm
LDFLAGS='-Wl,-s'
echo \"$FOOBAR\"
echo \"$PKGCONFIG\"
echo \"$XBPS_REMOVE_XCMD\"
echo \"$CLDFLAGS\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$PKGCONFIG", "$XBPS_REMOVE_XCMD", "$CLDFLAGS"]
        );
    }

    #[test]
    fn reports_underscore_split_variants() {
        let source = "\
#!/bin/sh
CT_ID=100
PKG_CONFIG=pkg-config
SKIPTEST=0
echo \"$CTID\"
echo \"$PKGCONFIG\"
echo \"$SKIP_TESTS\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$CTID", "$PKGCONFIG", "$SKIP_TESTS"]
        );
    }

    #[test]
    fn ignores_short_plural_compaction_segments() {
        let source = "\
#!/bin/sh
WIFIDEV=wlan0
echo \"$WIFI_DEVS\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_runtime_environment_names() {
        let source = "\
#!/bin/sh
path=1
echo \"$PATH\"
gem_home=1
gem_path=1
euid=1
echo \"$GEM_HOME\"
echo \"$GEM_PATH\"
echo \"$EUID\"
tmpdir=1
echo \"$TMPDIR\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_defaulting_parameter_operands() {
        let source = "\
#!/bin/sh
: \"${GENERIC_PACKS:=${GENERIC_PACK}}\"
EMAIL=${EMAIL:=$GMAIL}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_unguarded_references_before_later_defaulting_references() {
        let source = "\
#!/bin/sh
apkbin=apk
echo \"$APKBIN\"
echo \"${APKBIN:-apk}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$APKBIN"]
        );
    }

    #[test]
    fn reports_references_inside_expanding_heredocs() {
        let source = "\
#!/bin/sh
package_name=demo
cat << EOF
$PACKAGE_NAME
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$PACKAGE_NAME"]
        );
    }

    #[test]
    fn ignores_guarded_references_inside_expanding_heredocs() {
        let source = "\
#!/bin/sh
package_name=demo
cat << EOF
${PACKAGE_NAME:-demo}
EOF
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_presence_tested_names_inside_expanding_heredocs() {
        let source = "\
#!/bin/sh
cat << EOF
$INTERNAL_IP4_ADDRESS
EOF
if [ -n \"$INTERNAL_IP4_ADDRESS\" ]; then :; fi
if [ -n \"$INTERNAL_IP6_ADDRESS\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_transposed_common_build_settings_that_shellcheck_does_not_match() {
        let source = "\
#!/bin/sh
LDFLGAS='-Wl,--gc-sections'
echo \"$LDFLAGS\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_two_edit_environment_style_matches_and_short_references() {
        let source = "\
#!/bin/sh
CXXFLAGS='-O2'
DISK_REF=scsi0
OPT1=1
echo \"$CFLAGS\"
echo \"$DISK0_REF\"
echo \"$OPT\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$CFLAGS", "$DISK0_REF", "$OPT"]
        );
    }

    #[test]
    fn reports_build_flag_family_references_in_loop_headers() {
        let source = "\
#!/bin/bash
CXXFLAGS=\"${CXXFLAGS//-stdlib=libc++/}\"
for f in ${CFLAGS}; do
  echo \"c flag: ${f}\"
done
MY_CXXFLAGS=\"${MY_CXXFLAGS:-}\"
for f in ${MY_CFLAGS}; do
  echo \"custom c flag: ${f}\"
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}", "${MY_CFLAGS}"]
        );
    }

    #[test]
    fn reports_build_flag_scope_compat_with_presence_test_candidate() {
        let source = "\
#!/bin/bash
if [ -n \"${CXXFLAGS}\" ]; then :; fi
for f in ${CFLAGS}; do
  echo \"c flag: ${f}\"
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn reports_build_flag_scope_compat_with_unbraced_derived_candidate() {
        let source = "\
#!/bin/bash
for f in $CXXFLAGS; do
  echo \"cxx flag: ${f}\"
done
for f in ${CFLAGS}; do
  echo \"c flag: ${f}\"
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn reports_build_flag_scope_compat_with_parameter_candidate() {
        let source = "\
#!/bin/bash
tmp=\"${CXXFLAGS}\"
for f in ${CFLAGS}; do
  echo \"c flag: ${f}\"
done
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn reports_build_flag_scope_compat_with_quoted_derived_candidate() {
        let source = "\
#!/bin/bash
declare -r EXTRA_FLAGS=\"\\
$(
for f in \"${CXXFLAGS}\"; do
  echo \"cxx flag: ${f}\"
done
for f in ${CFLAGS}; do
  echo \"c flag: ${f}\"
done
)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn reports_build_flag_family_references_in_declaration_command_substitution_loop_headers() {
        let source = "\
#!/bin/bash
CXXFLAGS=\"${CXXFLAGS//-stdlib=libc++/}\"
declare -r EXTRA_FLAGS=\"\\
$(
for f in ${CFLAGS}; do
  echo \"c flag: ${f}\"
done
)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["${CFLAGS}"]
        );
    }

    #[test]
    fn reports_shellspec_execdir_case_reference() {
        let source = "\
#!/bin/sh
if [ ! \"$SHELLSPEC_PROJECT_ROOT\" ]; then
  case $SHELLSPEC_EXECDIR in (@basedir*)
    exit 1
  esac
fi
export SHELLSPEC_SPECDIR=\"$SHELLSPEC_HELPERDIR\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SHELLSPEC_EXECDIR"]
        );
    }

    #[test]
    fn reports_shellspec_execdir_with_presence_test_candidate() {
        let source = "\
#!/bin/sh
if [ -n \"$SHELLSPEC_SPECDIR\" ]; then :; fi
case $SHELLSPEC_EXECDIR in (@basedir*)
  exit 1
esac
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$SHELLSPEC_EXECDIR"]
        );
    }

    #[test]
    fn reports_xprefixed_candidates() {
        let source = "\
#!/bin/sh
XCPPFLAGS=1
XCFLAGS=1
XLDFLAGS=1
CXXFLAGS=1
echo \"$CPPFLAGS\"
echo \"$CFLAGS\"
echo \"$LDFLAGS\"
echo \"$CXXFLAGS\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$CPPFLAGS", "$CFLAGS", "$LDFLAGS"]
        );
    }

    #[test]
    fn reports_shellcheck_matched_prefix_and_runtime_candidate_pairs() {
        let source = "\
#!/bin/sh
HOSTID2=demo
HOSTNAME=demo
echo \"$HOSTID\"
echo \"$OS_NAME\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$HOSTID", "$OS_NAME"]
        );
    }

    #[test]
    fn reports_presence_tested_names_as_reference_candidates() {
        let source = "\
#!/bin/sh
echo \"$AWKBINARY\"
echo \"$TRBINARY\"
if [ -n \"$APKBINARY\" ]; then :; fi
if [ \"$IPBINARY\" ]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$AWKBINARY", "$TRBINARY"]
        );
    }

    #[test]
    fn reports_variable_set_tests_as_reference_candidates() {
        let source = "\
#!/bin/bash
echo \"$AWKBINARY\"
if [[ -v APKBINARY ]]; then :; fi
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$AWKBINARY"]
        );
    }

    #[test]
    fn reports_nested_presence_tested_names_as_reference_candidates() {
        let source = "\
#!/bin/sh
echo \"$AWKBINARY\"
: \"$(if [ -n \"$APKBINARY\" ]; then echo ok; fi)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$AWKBINARY"]
        );
    }

    #[test]
    fn ignores_plain_reference_only_candidate_names() {
        let source = "\
#!/bin/sh
echo \"$AWKBINARY\"
echo \"$APKBINARY\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_hostid_label_echoes() {
        let source = "\
#!/bin/sh
HOSTID2=demo
echo \"hostid=$HOSTID\"
${ECHOCMD} \"hostid=${HOSTID}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_hostid_in_unrelated_hostid_text_contexts() {
        let source = "\
#!/bin/sh
HOSTID2=demo
echo \"https://example.invalid/?hostid=$HOSTID\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$HOSTID"]
        );
    }

    #[test]
    fn ignores_references_when_the_exact_same_name_is_defined_elsewhere() {
        let source = "\
#!/bin/sh
f() { PACKAGE_NAME=demo; }
echo \"$PACKAGE_NAME\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_candidates_even_when_the_assigned_name_is_in_a_function_or_later() {
        let source = "\
#!/bin/sh
echo \"$PACKAGE_NAME\"
f() { package_name=demo; }
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$PACKAGE_NAME"]
        );
    }

    #[test]
    fn ignores_guarded_environment_knobs_and_runtime_names() {
        let source = "\
#!/bin/bash
hostname=demo
seconds=1
pipestatus=1
start_delay=1
WITH_cyrus=1
FIX_B=1
: \"${START_DELAY:-1}\"
: \"${WITH_CYRUS:-yes}\"
if [[ -v FIX_C ]]; then
  echo \"$FIX_C\"
fi
if [ -v FIX_D ]; then
  echo \"$FIX_D\"
fi
test -v FIX_E && echo \"$FIX_E\"
if [[ ! -v FIX_F ]]; then
  echo \"$FIX_F\"
fi
echo \"$HOSTNAME\"
echo \"$SECONDS\"
echo \"$PIPESTATUS\"
echo \"$START_DELAY\"
echo \"$WITH_CYRUS\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_synthetic_github_actions_placeholders() {
        let source = "\
#!/bin/sh
_SHUCK_GHA_1=1
echo \"$_SHUCK_GHA_2\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_assignment_value_variants_named_after_the_target() {
        let source = "\
#!/bin/bash
SRCNAM64=demo
SRCNAM=\"$SRCNAM32\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty());
    }
}
