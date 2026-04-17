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

    None
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
            | "GEM_HOME"
            | "GEM_PATH"
    ) || name.starts_with("LC_")
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
    fn ignores_runtime_environment_names() {
        let source = "\
#!/bin/sh
path=1
echo \"$PATH\"
gem_home=1
gem_path=1
echo \"$GEM_HOME\"
echo \"$GEM_PATH\"
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
