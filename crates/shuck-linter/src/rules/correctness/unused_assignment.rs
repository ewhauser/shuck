use std::collections::{HashMap, HashSet};

use shuck_semantic::{Binding, BindingAttributes, BindingKind, ScopeId};

use crate::{Checker, Rule, Violation};

type BindingFamilyKey = (Option<ScopeId>, String);

pub struct UnusedAssignment {
    pub name: String,
}

impl Violation for UnusedAssignment {
    fn rule() -> Rule {
        Rule::UnusedAssignment
    }

    fn message(&self) -> String {
        format!("variable `{}` is assigned but never used", self.name)
    }
}

pub fn unused_assignment(checker: &mut Checker) {
    let semantic = checker.semantic();
    let unused_bindings = checker.semantic_analysis().unused_assignments();
    let unused_binding_ids = unused_bindings.iter().copied().collect::<HashSet<_>>();
    let mut families_with_used_reportable_bindings = HashSet::new();
    let mut unused_bindings_by_family = HashMap::<BindingFamilyKey, Vec<_>>::new();
    let mut last_unused_binding_by_family = HashMap::new();

    for binding in semantic.bindings() {
        if binding.name.as_str() == "_" {
            continue;
        }

        if !is_reportable_unused_assignment(binding.kind, binding.attributes) {
            continue;
        }

        if !unused_binding_ids.contains(&binding.id) {
            families_with_used_reportable_bindings.insert(binding_family_key(binding));
        }
    }

    for binding_id in unused_bindings {
        let binding = semantic.binding(*binding_id);
        if binding.name.as_str() == "_" {
            continue;
        }

        if !is_reportable_unused_assignment(binding.kind, binding.attributes) {
            continue;
        }

        let family = binding_family_key(binding);

        unused_bindings_by_family
            .entry(family.clone())
            .or_default()
            .push(*binding_id);

        last_unused_binding_by_family
            .entry(family)
            .and_modify(|current_binding_id| {
                let current = semantic.binding(*current_binding_id);
                if binding_follows_in_source(
                    current.span.start.offset,
                    current.span.end.offset,
                    binding.span.start.offset,
                    binding.span.end.offset,
                ) {
                    *current_binding_id = *binding_id;
                }
            })
            .or_insert(*binding_id);
    }

    let mut reportable_bindings = Vec::new();
    for (family, binding_ids) in unused_bindings_by_family {
        if families_with_used_reportable_bindings.contains(&family) {
            reportable_bindings.extend(binding_ids);
            continue;
        }

        if let Some(binding_id) = last_unused_binding_by_family.get(&family).copied() {
            reportable_bindings.push(binding_id);
        }
    }
    reportable_bindings
        .sort_unstable_by_key(|binding_id| semantic.binding(*binding_id).span.start.offset);

    for binding_id in reportable_bindings {
        let binding = semantic.binding(binding_id);

        if !is_reportable_unused_assignment(binding.kind, binding.attributes) {
            continue;
        }

        // Exported variables are consumed by child processes.
        if binding.attributes.contains(BindingAttributes::EXPORTED) {
            continue;
        }

        // Namerefs redirect to another variable; the binding itself is not
        // a conventional assignment.
        if matches!(binding.kind, BindingKind::Nameref) {
            continue;
        }

        checker.report(
            UnusedAssignment {
                name: binding.name.to_string(),
            },
            binding.span,
        );
    }
}

fn is_reportable_unused_assignment(kind: BindingKind, attributes: BindingAttributes) -> bool {
    match kind {
        BindingKind::Assignment
        | BindingKind::ArrayAssignment
        | BindingKind::LoopVariable
        | BindingKind::ReadTarget
        | BindingKind::MapfileTarget
        | BindingKind::PrintfTarget
        | BindingKind::GetoptsTarget
        | BindingKind::ArithmeticAssignment => true,
        BindingKind::AppendAssignment | BindingKind::ParameterDefaultAssignment => false,
        BindingKind::Declaration(_) => {
            attributes.contains(BindingAttributes::DECLARATION_INITIALIZED)
        }
        BindingKind::FunctionDefinition | BindingKind::Imported | BindingKind::Nameref => false,
    }
}

fn binding_follows_in_source(
    current_start: usize,
    current_end: usize,
    candidate_start: usize,
    candidate_end: usize,
) -> bool {
    candidate_start > current_start
        || (candidate_start == current_start && candidate_end > current_end)
}

fn binding_family_key(binding: &Binding) -> BindingFamilyKey {
    // `local`-scoped bindings should not collapse across functions, but plain
    // assignments still participate in the wider variable family even when
    // they appear inside a function body.
    let scope = binding
        .attributes
        .contains(BindingAttributes::LOCAL)
        .then_some(binding.scope);

    (scope, binding.name.to_string())
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_variable_name_span() {
        let source = "#!/bin/sh\nunused=1\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "unused");
    }

    #[test]
    fn reports_only_the_last_unused_binding_for_a_name() {
        let source = "#!/bin/sh\nfoo=1\nfoo=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
        assert_eq!(diagnostics[0].span.slice(source), "foo");
    }

    #[test]
    fn uses_source_order_when_function_bindings_share_a_name() {
        let source = "#!/bin/bash\nf(){ foo=1; }\nfoo=2\nf\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 3);
    }

    #[test]
    fn later_exports_suppress_the_name_family() {
        let source = "#!/bin/sh\nfoo=1\nexport foo=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn keeps_distinct_local_scopes_separate() {
        let source = "#!/bin/bash\nf(){ local foo=1; }\ng(){ local foo=2; }\nf\ng\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 2);
        assert_eq!(diagnostics[0].span.start.line, 2);
        assert_eq!(diagnostics[1].span.start.line, 3);
    }

    #[test]
    fn later_non_reportable_bindings_do_not_hide_earlier_assignments() {
        let source = "#!/bin/bash\nfoo=1\nfoo+=2\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::UnusedAssignment));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.start.line, 2);
    }
}
