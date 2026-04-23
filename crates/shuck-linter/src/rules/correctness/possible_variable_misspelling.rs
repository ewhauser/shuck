use rustc_hash::FxHashSet;

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
    let guarded_names = checker
        .semantic()
        .references()
        .iter()
        .filter(|reference| {
            checker
                .semantic()
                .is_guarded_parameter_reference(reference.id)
        })
        .map(|reference| reference.name.clone())
        .collect::<FxHashSet<_>>();

    let mut findings = checker
        .semantic()
        .unresolved_references()
        .iter()
        .copied()
        .filter_map(|reference_id| {
            let reference = checker.semantic().reference(reference_id);
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
            if guarded_names.contains(&reference.name) {
                return None;
            }
            if has_same_name_defining_bindings(checker, &reference.name) {
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
            if reference_is_source_prefix_of_candidate(
                checker,
                reference.name.as_str(),
                reference.span,
                candidate.as_str(),
            ) {
                return None;
            }
            Some((reference.span, reference.name.to_string(), candidate))
        })
        .collect::<Vec<_>>();

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
}

fn canonical_uppercase_name(name: &str) -> String {
    name.chars().map(|char| char.to_ascii_uppercase()).collect()
}

fn candidate_match_rank(target_name: &str, candidate_name: &str) -> Option<u8> {
    let candidate_upper = canonical_uppercase_name(candidate_name);

    if target_name.len() >= 4 && candidate_upper == target_name {
        return Some(0);
    }

    if !is_environment_style_name(candidate_name)
        || target_name.len() < 3
        || candidate_name.len() < 4
    {
        return None;
    }

    let distance =
        bounded_ascii_edit_distance(target_name.as_bytes(), candidate_upper.as_bytes(), 2)?;
    if distance == 0 {
        return None;
    }
    if distance == 2 && !has_strong_two_edit_shape(target_name, candidate_upper.as_str()) {
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

    is_compiler_flag_family_pair(target_name, candidate_upper)
        || common_prefix >= 5
        || common_suffix >= 6
        || (common_prefix >= 4 && common_suffix >= 4)
        || (common_prefix >= 2 && common_suffix >= 5)
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

fn reference_is_source_prefix_of_candidate(
    checker: &Checker<'_>,
    reference_name: &str,
    reference_span: shuck_ast::Span,
    candidate_name: &str,
) -> bool {
    let candidate_upper = canonical_uppercase_name(candidate_name);
    let Some(suffix) = candidate_upper.strip_prefix(reference_name) else {
        return false;
    };
    if suffix.is_empty() || !suffix.chars().all(|char| char.is_ascii_digit()) {
        return false;
    }

    checker
        .source()
        .as_bytes()
        .get(reference_span.end.offset..reference_span.end.offset + suffix.len())
        .is_some_and(|source_suffix| source_suffix.eq_ignore_ascii_case(suffix.as_bytes()))
}

fn is_compiler_flag_family_pair(target_name: &str, candidate_upper: &str) -> bool {
    target_name == "CFLAGS" && matches!(candidate_upper, "CPPFLAGS" | "CXXFLAGS" | "CLDFLAGS")
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

fn bounded_ascii_edit_distance(left: &[u8], right: &[u8], max_distance: u8) -> Option<u8> {
    let max_distance = usize::from(max_distance);
    if left.len().abs_diff(right.len()) > max_distance {
        return None;
    }

    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];

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
echo \"$CTID\"
echo \"$PKGCONFIG\"
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
            vec!["$CTID", "$PKGCONFIG"]
        );
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
    fn ignores_transposed_common_build_settings() {
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
