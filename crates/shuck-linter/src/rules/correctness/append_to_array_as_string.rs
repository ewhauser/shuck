use shuck_semantic::{BindingAttributes, BindingKind};

use crate::facts::words::leading_literal_word_prefix;
use crate::{Checker, Diagnostic, Edit, Fix, FixAvailability, Rule, Violation};

pub struct AppendToArrayAsString;

impl Violation for AppendToArrayAsString {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::Always;

    fn rule() -> Rule {
        Rule::AppendToArrayAsString
    }

    fn message(&self) -> String {
        "appending a string to an array with `+=` merges into an element; use `+=(...)`".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        Some("append a new array element".to_owned())
    }
}

pub fn append_to_array_as_string(checker: &mut Checker) {
    let source = checker.source();
    let semantic = checker.semantic();

    let diagnostics = semantic
        .bindings()
        .iter()
        .filter_map(|binding| {
            if binding.kind != BindingKind::AppendAssignment {
                return None;
            }
            if binding
                .attributes
                .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
            {
                return None;
            }

            let prior_binding = semantic.previous_visible_binding(
                &binding.name,
                binding.span,
                Some(binding.span),
            )?;
            if !semantic.binding_has_array_value_shape(prior_binding.id) {
                return None;
            }

            let value = checker.facts().binding_value(binding.id)?.scalar_word()?;
            if !leading_literal_word_prefix(value, source).starts_with(' ') {
                return None;
            }

            Some((
                binding.span,
                Fix::unsafe_edits([
                    Edit::insertion(value.span.start.offset, "("),
                    Edit::insertion(value.span.end.offset, ")"),
                ]),
            ))
        })
        .collect::<Vec<_>>();

    for (span, fix) in diagnostics {
        checker.report_diagnostic_dedup(Diagnostic::new(AppendToArrayAsString, span).with_fix(fix));
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path_with_fix, test_snippet, test_snippet_with_fix};
    use crate::{Applicability, LinterSettings, Rule, assert_diagnostics_diff};

    #[test]
    fn reports_string_appends_on_array_bindings() {
        let source = "\
#!/bin/bash
items=(one)
items+=\" two\"
declare -a flags=(--first)
flags+=\" ${extra}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendToArrayAsString),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["items", "flags"]
        );
    }

    #[test]
    fn ignores_non_array_and_element_appends() {
        let source = "\
#!/bin/bash
name=base
name+=\" suffix\"
items=(one)
items+=(\" two\")
items[0]+=\" tail\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendToArrayAsString),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_shadowed_local_scalars_when_outer_binding_is_array() {
        let source = "\
#!/bin/bash
arr=(one)
f() {
  local arr=base
  arr+=\" two\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::AppendToArrayAsString),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn applies_unsafe_fix_to_string_appends_on_array_bindings() {
        let source = "\
#!/bin/bash
items=(one)
items+=\" two\"
declare -a flags=(--first)
flags+=\" ${extra}\"
";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AppendToArrayAsString),
            Applicability::Unsafe,
        );

        assert_eq!(result.fixes_applied, 2);
        assert_eq!(
            result.fixed_source,
            "\
#!/bin/bash
items=(one)
items+=(\" two\")
declare -a flags=(--first)
flags+=(\" ${extra}\")
"
        );
        assert!(result.fixed_diagnostics.is_empty());
    }

    #[test]
    fn safe_fix_mode_leaves_string_appends_unchanged() {
        let source = "#!/bin/bash\nitems=(one)\nitems+=\" two\"\n";
        let result = test_snippet_with_fix(
            source,
            &LinterSettings::for_rule(Rule::AppendToArrayAsString),
            Applicability::Safe,
        );

        assert_eq!(result.fixes_applied, 0);
        assert_eq!(result.fixed_source, source);
        assert_eq!(result.fixed_diagnostics.len(), 1);
    }

    #[test]
    fn snapshots_unsafe_fix_output_for_fixture() -> anyhow::Result<()> {
        let result = test_path_with_fix(
            Path::new("correctness").join("C106.sh").as_path(),
            &LinterSettings::for_rule(Rule::AppendToArrayAsString),
            Applicability::Unsafe,
        )?;

        assert_diagnostics_diff!("C106_fix_C106.sh", result);
        Ok(())
    }
}
