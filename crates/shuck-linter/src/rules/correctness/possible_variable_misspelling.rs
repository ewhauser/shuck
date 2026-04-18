use rustc_hash::FxHashSet;
use shuck_semantic::Binding;

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
            if guarded_names.contains(&reference.name) {
                return None;
            }
            if has_same_name_defining_bindings(checker, &reference.name) {
                return None;
            }

            let candidate = preferred_candidate_binding(checker, reference.name.as_str())?;
            Some((
                reference.span,
                reference.name.to_string(),
                candidate.name.to_string(),
            ))
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
        && name.len() >= 4
        && name.chars().any(|char| char.is_ascii_uppercase())
}

fn preferred_candidate_binding<'a>(
    checker: &'a Checker<'_>,
    target_name: &str,
) -> Option<&'a Binding> {
    let candidates = checker
        .semantic()
        .bindings()
        .iter()
        .filter(|binding| is_sc2154_defining_binding(binding.kind))
        .filter(|binding| binding.name.as_str() != target_name)
        .filter(|binding| binding.name.as_str().len() >= 4)
        .filter_map(|binding| {
            candidate_match_rank(target_name, binding.name.as_str()).map(|rank| (rank, binding))
        })
        .collect::<Vec<_>>();

    let best_rank = candidates.iter().map(|(rank, _)| *rank).min()?;
    let unique_best_candidates = candidates
        .iter()
        .filter(|(rank, _)| *rank == best_rank)
        .map(|(_, binding)| canonical_uppercase_name(binding.name.as_str()))
        .collect::<FxHashSet<_>>();
    if unique_best_candidates.len() > 1 {
        return None;
    }

    candidates
        .into_iter()
        .filter(|(rank, _)| *rank == best_rank)
        .min_by_key(|(_, binding)| (binding.span.start.offset, binding.span.end.offset))
        .map(|(_, binding)| binding)
}

fn canonical_uppercase_name(name: &str) -> String {
    name.chars().map(|char| char.to_ascii_uppercase()).collect()
}

fn candidate_match_rank(target_name: &str, candidate_name: &str) -> Option<u8> {
    let candidate_upper = canonical_uppercase_name(candidate_name);
    if candidate_upper == target_name {
        return Some(0);
    }
    if candidate_upper == format!("X{target_name}") {
        return Some(1);
    }
    if target_name == format!("{candidate_upper}_") {
        return Some(2);
    }
    if is_short_split_id_variant(target_name, candidate_upper.as_str()) {
        return Some(3);
    }
    if is_common_build_setting_name(target_name) {
        return None;
    }
    if is_environment_style_name(candidate_name)
        && has_single_environment_style_typo(target_name, candidate_upper.as_str())
    {
        return Some(4);
    }

    None
}

fn is_short_split_id_variant(target_name: &str, candidate_upper: &str) -> bool {
    short_two_segment_underscore_variant(candidate_upper, target_name)
        || short_two_segment_underscore_variant(target_name, candidate_upper)
}

fn short_two_segment_underscore_variant(with_underscore: &str, without_underscore: &str) -> bool {
    let mut segments = with_underscore.split('_');
    let Some(first) = segments.next() else {
        return false;
    };
    let Some(second) = segments.next() else {
        return false;
    };
    if segments.next().is_some() || first.is_empty() || second.is_empty() {
        return false;
    }
    if first.len() > 2 || second.len() > 2 {
        return false;
    }

    format!("{first}{second}") == without_underscore
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EnvironmentStyleEdit {
    Substitute,
    Insert { byte: u8, index: usize },
    Delete { byte: u8, index: usize },
    Transpose,
}

fn has_single_environment_style_typo(target_name: &str, candidate_upper: &str) -> bool {
    let Some(edit) =
        single_environment_style_edit(target_name.as_bytes(), candidate_upper.as_bytes())
    else {
        return false;
    };

    match edit {
        EnvironmentStyleEdit::Substitute | EnvironmentStyleEdit::Transpose => true,
        EnvironmentStyleEdit::Insert { byte, index }
        | EnvironmentStyleEdit::Delete { byte, index } => {
            index > 0
                && !(byte == b'X'
                    && byte_before_edit(index, target_name.as_bytes(), candidate_upper.as_bytes())
                        == Some(b'_'))
        }
    }
}

fn single_environment_style_edit(
    target: &[u8],
    candidate: &[u8],
) -> Option<EnvironmentStyleEdit> {
    if target.len() == candidate.len() {
        let mismatches = target
            .iter()
            .zip(candidate.iter())
            .enumerate()
            .filter_map(|(index, (&left, &right))| (left != right).then_some((index, left, right)))
            .collect::<Vec<_>>();

        return match mismatches.as_slice() {
            [(_, left, right)] if left.is_ascii_alphabetic() && right.is_ascii_alphabetic() => {
                Some(EnvironmentStyleEdit::Substitute)
            }
            [(first_index, first_left, first_right), (second_index, second_left, second_right)]
                if second_index == &(first_index + 1)
                    && first_left.is_ascii_alphabetic()
                    && first_right.is_ascii_alphabetic()
                    && second_left.is_ascii_alphabetic()
                    && second_right.is_ascii_alphabetic()
                    && *first_left == *second_right
                    && *second_left == *first_right =>
            {
                Some(EnvironmentStyleEdit::Transpose)
            }
            _ => None,
        };
    }

    if target.len() + 1 == candidate.len() {
        let index = first_mismatch_index(target, candidate).unwrap_or(target.len());
        let inserted = candidate[index];
        return (inserted.is_ascii_alphabetic() && target[index..] == candidate[index + 1..])
            .then_some(EnvironmentStyleEdit::Insert {
                byte: inserted,
                index,
            });
    }

    if candidate.len() + 1 == target.len() {
        let index = first_mismatch_index(target, candidate).unwrap_or(candidate.len());
        let deleted = target[index];
        return (deleted.is_ascii_alphabetic() && target[index + 1..] == candidate[index..])
            .then_some(EnvironmentStyleEdit::Delete {
                byte: deleted,
                index,
            });
    }

    None
}

fn first_mismatch_index(left: &[u8], right: &[u8]) -> Option<usize> {
    left.iter()
        .zip(right.iter())
        .position(|(left, right)| left != right)
}

fn byte_before_edit(index: usize, target: &[u8], candidate: &[u8]) -> Option<u8> {
    index
        .checked_sub(1)
        .and_then(|previous| target.get(previous).or_else(|| candidate.get(previous)))
        .copied()
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
            | "GEM_HOME"
            | "GEM_PATH"
    ) || name.starts_with("LC_")
}

fn is_common_build_setting_name(name: &str) -> bool {
    matches!(name, "CFLAGS" | "CPPFLAGS" | "CXXFLAGS" | "LDFLAGS" | "LIBS")
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
    fn keeps_reviewed_alias_families_out_of_scope() {
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

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_short_split_id_variants_but_not_broader_underscore_aliases() {
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
            vec!["$CTID"]
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
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::PossibleVariableMisspelling),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_fuzzy_matches_for_common_build_settings() {
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
: \"${START_DELAY:-1}\"
: \"${WITH_CYRUS:-yes}\"
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
}
