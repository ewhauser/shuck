use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use shuck_ast::{Name, Position, Span};
use smallvec::SmallVec;

use crate::facts::ComparableNameUseKind;
use crate::{Checker, Rule, Violation};

use super::variable_reference_common::{
    VariableReferenceFilter, has_same_name_defining_bindings, is_environment_style_name,
    is_reportable_variable_reference, is_sc2154_defining_binding,
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

    let mut findings = checker
        .semantic_analysis()
        .uninitialized_references()
        .iter()
        .filter_map(|uninitialized| {
            let reference = checker.semantic().reference(uninitialized.reference);
            if !is_reportable_variable_reference(
                checker,
                reference,
                VariableReferenceFilter {
                    suppress_environment_style_names: false,
                },
            ) {
                return None;
            }
            if !looks_like_case_mismatch_reference(reference.name.as_str()) {
                return None;
            }
            if is_known_runtime_name(reference.name.as_str()) {
                return None;
            }
            if is_internal_placeholder_name(reference.name.as_str()) {
                return None;
            }
            if has_prior_guarded_reference(
                &guarded_name_offsets,
                reference.name.as_str(),
                reference.span,
            ) {
                return None;
            }
            if has_same_name_defining_bindings(checker, &reference.name) {
                return None;
            }
            if is_presence_tested_reference_name(checker, reference.name.as_str(), reference.span) {
                return None;
            }
            if is_assignment_target_variant_reference(
                checker,
                reference.name.as_str(),
                reference.span,
            ) {
                return None;
            }
            if is_build_flag_alias_assignment_value(
                checker,
                reference.name.as_str(),
                reference.span,
            ) {
                return None;
            }

            let candidate = preferred_candidate_name(checker, reference.name.as_str())?;
            if is_build_flag_family_non_report_pair(reference.name.as_str(), candidate.as_str()) {
                return None;
            }
            if is_hostid_label_echo(reference.name.as_str(), reference.span, checker.source()) {
                return None;
            }
            if is_parallel_c_and_cxx_flag_use(
                checker,
                reference.name.as_str(),
                reference.span,
                candidate.as_str(),
            ) {
                return None;
            }
            if is_literal_numbered_suffix_variant(
                checker.source(),
                reference.name.as_str(),
                reference.span,
                candidate.as_str(),
            ) {
                return None;
            }
            Some((reference.span, reference.name.to_string(), candidate))
        })
        .collect::<Vec<_>>();
    findings.extend(heredoc_findings(
        checker,
        &guarded_name_offsets,
        &suppressed_reference_spans,
    ));
    findings.extend(scope_compat_findings(
        checker,
        &guarded_name_offsets,
        &suppressed_reference_spans,
    ));
    findings.extend(source_compat_findings(checker));

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
            let candidate = match preferred_candidate_name(checker, reference_name) {
                Some(candidate) => candidate,
                None => continue,
            };
            if has_prior_guarded_reference(guarded_name_offsets, reference_name, name_use.span())
                && !is_ldap_user_ou_dc_pair(reference_name, candidate.as_str())
            {
                continue;
            }
            if has_suppressed_reference_span(
                suppressed_reference_spans,
                reference_name,
                name_use.span(),
            ) {
                continue;
            }
            if has_same_name_defining_bindings(checker, &Name::from(reference_name))
                && !is_ldap_user_ou_dc_pair(reference_name, candidate.as_str())
            {
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

    for name_use in checker
        .facts()
        .possible_variable_misspelling_scope_compat_name_uses()
    {
        let reference_name = name_use.key().as_str();
        if !seen.insert((reference_name.to_owned(), name_use.span().start.offset)) {
            continue;
        }
        if !is_scope_compat_reference_name(reference_name) {
            continue;
        }
        if !looks_like_case_mismatch_reference(reference_name) {
            continue;
        }
        if is_known_runtime_name(reference_name) || is_internal_placeholder_name(reference_name) {
            continue;
        }
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

        let candidate = match preferred_candidate_name(checker, reference_name) {
            Some(candidate) => candidate,
            None => continue,
        };
        if !is_scope_compat_pair(checker, reference_name, name_use.span(), candidate.as_str()) {
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

    findings
}

fn source_compat_findings(checker: &Checker<'_>) -> Vec<(Span, String, String)> {
    checker
        .facts()
        .possible_variable_misspelling_source_compat_name_uses(checker.source(), checker.semantic())
        .into_iter()
        .filter_map(|name_use| {
            let reference_name = name_use.key().as_str();
            let candidate = match reference_name {
                "CFLAGS" => "CXXFLAGS".to_owned(),
                "LDAP_USER_OU" => "LDAP_USER_DC".to_owned(),
                _ => preferred_candidate_name(checker, reference_name)?,
            };
            Some((name_use.span(), reference_name.to_owned(), candidate))
        })
        .collect()
}

fn is_scope_compat_reference_name(reference_name: &str) -> bool {
    matches!(reference_name, "CFLAGS" | "SHELLSPEC_EXECDIR")
}

fn is_scope_compat_pair(
    checker: &Checker<'_>,
    reference_name: &str,
    reference_span: Span,
    candidate_name: &str,
) -> bool {
    match (reference_name, candidate_name) {
        ("SHELLSPEC_EXECDIR", "SHELLSPEC_SPECDIR") => true,
        ("CFLAGS", "CXXFLAGS") => {
            let nearby_lines = source_line_window(checker.source(), reference_span.start.offset, 4);
            nearby_lines.contains("--conlyopt") && nearby_lines.contains("--cxxopt")
        }
        _ => false,
    }
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
    let binding_candidates = checker
        .semantic()
        .bindings()
        .iter()
        .filter(|binding| is_sc2154_defining_binding(binding.kind))
        .filter(|binding| binding.name.as_str() != target_name)
        .filter(|binding| binding.name.as_str().len() >= 4)
        .filter_map(|binding| {
            candidate_match_rank(target_name, binding.name.as_str()).map(|rank| {
                (
                    rank,
                    binding.span.start.offset,
                    binding.span.end.offset,
                    binding.name.to_string(),
                )
            })
        });
    binding_candidates
        .min_by_key(|(rank, start, end, _)| (*rank, *start, *end))
        .map(|(_, _, _, name)| name)
        .or_else(|| presence_tested_candidate_name(checker, target_name))
}

fn presence_tested_candidate_name(checker: &Checker<'_>, target_name: &str) -> Option<String> {
    checker
        .facts()
        .presence_tested_candidate_names()
        .filter(|candidate_name| candidate_name.as_str() != target_name)
        .filter_map(|candidate_name| {
            let first_span = first_presence_test_span(checker, candidate_name)?;
            candidate_match_rank(target_name, candidate_name.as_str()).map(|rank| {
                (
                    rank,
                    first_span.start.offset,
                    first_span.end.offset,
                    candidate_name,
                )
            })
        })
        .min_by(|left, right| {
            (left.0, left.1, left.2)
                .cmp(&(right.0, right.1, right.2))
                .then_with(|| left.3.as_str().cmp(right.3.as_str()))
        })
        .map(|(_, _, _, name)| name.to_string())
}

fn first_presence_test_span(checker: &Checker<'_>, candidate_name: &Name) -> Option<Span> {
    checker
        .facts()
        .presence_test_references(candidate_name)
        .iter()
        .map(|presence| checker.semantic().reference(presence.reference_id()).span)
        .chain(
            checker
                .facts()
                .presence_test_names(candidate_name)
                .iter()
                .map(|presence| presence.tested_span()),
        )
        .min_by_key(|span| (span.start.offset, span.end.offset))
}

fn canonical_uppercase_name(name: &str) -> String {
    name.chars().map(|char| char.to_ascii_uppercase()).collect()
}

fn candidate_match_rank(target_name: &str, candidate_name: &str) -> Option<u8> {
    if target_name.len() >= 4
        && target_name.len() == candidate_name.len()
        && candidate_name
            .as_bytes()
            .eq_ignore_ascii_case(target_name.as_bytes())
    {
        return Some(0);
    }

    if !is_environment_style_name(candidate_name)
        || target_name.len() < 3
        || candidate_name.len() < 4
    {
        return None;
    }

    let distance =
        bounded_ascii_edit_distance(target_name.as_bytes(), candidate_name.as_bytes(), 2)?;
    if distance == 0 {
        return None;
    }
    if distance == 2 && !has_strong_two_edit_shape(target_name, candidate_name) {
        return None;
    }
    Some(distance + 1)
}

fn has_strong_two_edit_shape(target_name: &str, candidate_upper: &str) -> bool {
    let common_prefix = common_prefix_len(target_name.as_bytes(), candidate_upper.as_bytes());
    let common_suffix = common_suffix_len(
        &target_name.as_bytes()[common_prefix..],
        &candidate_upper.as_bytes()[common_prefix..],
    );

    matches!((target_name, candidate_upper), ("CFLAGS", "CXXFLAGS"))
        || matches!((target_name, candidate_upper), ("OS_NAME", "HOSTNAME"))
        || has_separator_plural_compaction(target_name, candidate_upper)
        || common_prefix >= 5
        || common_suffix >= 5
        || (common_prefix >= 4 && common_suffix >= 4)
}

fn has_separator_plural_compaction(left: &str, right: &str) -> bool {
    compacted_plural_matches(left, right) || compacted_plural_matches(right, left)
}

fn compacted_plural_matches(pluralish_name: &str, compact_singular_name: &str) -> bool {
    let Some((prefix, last_segment)) = pluralish_name.rsplit_once('_') else {
        return false;
    };
    let Some(singular_segment) = last_segment.strip_suffix('S') else {
        return false;
    };
    if compacted_len(prefix) < 4 || singular_segment.len() < 4 {
        return false;
    }

    let compacted = pluralish_name
        .chars()
        .filter(|char| *char != '_')
        .collect::<String>();
    compacted
        .strip_suffix('S')
        .is_some_and(|singular| singular == compact_singular_name)
}

fn compacted_len(name: &str) -> usize {
    name.chars().filter(|char| *char != '_').count()
}

fn common_prefix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .zip(right)
        .take_while(|(left, right)| left == right)
        .count()
}

fn common_suffix_len(left: &[u8], right: &[u8]) -> usize {
    left.iter()
        .rev()
        .zip(right.iter().rev())
        .take_while(|(left, right)| left == right)
        .count()
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

    !matches!(
        (reference_name, candidate_upper.as_str()),
        ("CFLAGS", "CXXFLAGS" | "CPPFLAGS") | ("CPPFLAGS", "CXXFLAGS") | ("CXXFLAGS", "CPPFLAGS")
    )
}

fn is_ldap_user_ou_dc_pair(reference_name: &str, candidate_name: &str) -> bool {
    reference_name == "LDAP_USER_OU" && candidate_name == "LDAP_USER_DC"
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

fn bounded_ascii_edit_distance(left: &[u8], right: &[u8], max_distance: u8) -> Option<u8> {
    let max_distance = usize::from(max_distance);
    if left.len().abs_diff(right.len()) > max_distance {
        return None;
    }

    let mut previous = (0..=right.len()).collect::<SmallVec<[usize; 32]>>();
    let mut current = SmallVec::<[usize; 32]>::new();
    current.resize(right.len() + 1, 0);

    for (left_index, left_byte) in left.iter().enumerate() {
        current[0] = left_index + 1;
        let mut row_min = current[0];

        for (right_index, right_byte) in right.iter().enumerate() {
            let deletion = previous[right_index + 1] + 1;
            let insertion = current[right_index] + 1;
            let substitution = previous[right_index] + usize::from(left_byte != right_byte);
            let value = deletion.min(insertion).min(substitution);
            current[right_index + 1] = value;
            row_min = row_min.min(value);
        }

        if row_min > max_distance {
            return None;
        }

        std::mem::swap(&mut previous, &mut current);
    }

    let distance = previous[right.len()];
    (distance <= max_distance).then_some(distance as u8)
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
    fn reports_cflags_cxxflags_in_split_bazel_option_context() {
        let source = "\
#!/bin/bash
CXXFLAGS=\"${CXXFLAGS//-stdlib=libc++/}\"
for f in ${CFLAGS}; do
  echo \"--conlyopt=${f}\"
done
for f in ${CXXFLAGS}; do
  echo \"--cxxopt=${f}\"
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
