use std::collections::HashMap;

use shuck_semantic::{Binding, BindingAttributes, BindingKind, DeclarationBuiltin};

use crate::{Checker, Rule, Violation};

pub struct ArrayToStringConversion;

impl Violation for ArrayToStringConversion {
    fn rule() -> Rule {
        Rule::ArrayToStringConversion
    }

    fn message(&self) -> String {
        "a variable name switches from array-like use to a plain scalar assignment".to_owned()
    }
}

pub fn array_to_string_conversion(checker: &mut Checker) {
    let semantic = checker.semantic();
    let mut array_history = HashMap::new();
    let mut bindings = semantic.bindings().iter().collect::<Vec<_>>();
    bindings.sort_by_key(|binding| (binding.span.start.offset, binding.span.end.offset));

    let spans = bindings
        .into_iter()
        .filter_map(|binding| {
            let name = binding.name.clone();
            let saw_array_history = array_history
                .get(&name)
                .copied()
                .unwrap_or_else(|| binding_uses_builtin_array_history(binding));

            if declaration_resets_array_history(binding) {
                array_history.insert(name, false);
                return None;
            }
            if !binding_can_trigger_array_to_string_conversion(binding) {
                if binding_establishes_array_history(checker, binding) {
                    array_history.insert(name, true);
                }
                return None;
            }
            if binding_is_array_like(binding) {
                if binding_establishes_array_history(checker, binding) {
                    array_history.insert(name, true);
                }
                return None;
            }

            checker.facts().binding_value(binding.id)?.scalar_word()?;

            if binding_establishes_array_history(checker, binding) {
                array_history.insert(name.clone(), true);
            }

            saw_array_history.then_some(binding.span)
        })
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || ArrayToStringConversion);
}

fn binding_can_trigger_array_to_string_conversion(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::Assignment
            | BindingKind::ParameterDefaultAssignment
            | BindingKind::Declaration(_)
    )
}

fn binding_establishes_array_history(checker: &Checker<'_>, binding: &Binding) -> bool {
    match binding.kind {
        BindingKind::Imported => false,
        BindingKind::ReadTarget => read_target_is_array_like(checker, binding),
        BindingKind::MapfileTarget => true,
        BindingKind::Declaration(DeclarationBuiltin::Local)
            if !binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED) =>
        {
            false
        }
        _ => binding_is_array_like(binding),
    }
}

fn declaration_resets_array_history(binding: &Binding) -> bool {
    match binding.kind {
        BindingKind::Declaration(DeclarationBuiltin::Local) => !binding
            .attributes
            .contains(BindingAttributes::DECLARATION_INITIALIZED),
        BindingKind::Declaration(DeclarationBuiltin::Declare | DeclarationBuiltin::Typeset) => {
            !binding
                .attributes
                .contains(BindingAttributes::DECLARATION_INITIALIZED)
                && !binding_is_array_like(binding)
        }
        _ => false,
    }
}

fn binding_uses_builtin_array_history(binding: &Binding) -> bool {
    matches!(binding.name.as_str(), "MAPFILE")
}

fn read_target_is_array_like(checker: &Checker<'_>, binding: &Binding) -> bool {
    checker
        .facts()
        .innermost_command_at(binding.span.start.offset)
        .or_else(|| {
            checker
                .facts()
                .commands()
                .iter()
                .rev()
                .find(|command| contains_span(command.span(), binding.span))
        })
        .and_then(|command| command.options().read())
        .is_some_and(|read| {
            read.array_target_name_uses()
                .iter()
                .any(|target| target.span() == binding.span)
        })
}

fn contains_span(outer: shuck_ast::Span, inner: shuck_ast::Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn binding_is_array_like(binding: &Binding) -> bool {
    binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        || binding.kind == BindingKind::ArrayAssignment
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_scalar_reassignments_after_prior_array_bindings() {
        let source = "\
#!/bin/bash
exts=(txt pdf doc)
exts=\"${exts[*]}\"
items=(one two)
items=\"${items[0]}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["exts", "items"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_assignments_without_prior_array_like_binding() {
        let source = "\
#!/bin/bash
name=base
name=\"${name}-suffix\"
other=\"${unknown:-fallback}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_shadowed_local_scalars_after_prior_array_bindings() {
        let source = "\
#!/bin/bash
exts=(txt pdf)
f() {
  local exts=base
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["exts"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn reports_scalar_declarations_after_prior_array_declarations() {
        let source = "\
#!/bin/bash
f() {
  declare -a cmd
  cmd=\"curl\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["cmd"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_assignments_after_bare_local_resets() {
        let source = "\
#!/bin/bash
exts=(txt pdf)
f() {
  local exts
  exts=base
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_bare_local_array_declarations_without_initializers() {
        let source = "\
#!/bin/bash
f() {
  local -a cmd
  local cmd=\"curl\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_scalar_reassignments_after_read_array_targets() {
        let source = "\
#!/bin/bash
f() {
  read -r -a resolution <<< \"1 2 3\"
  resolution=\"${resolution[0]} x ${resolution[1]} @ ${resolution[2]} fps\"
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["resolution"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn reports_global_scalar_reassignments_after_function_local_array_use() {
        let source = "\
#!/bin/bash
f() {
  local fuzzer=$1
  if [[ $fuzzer == *\"@\"* ]]; then
    fuzzer=(${fuzzer//@/ }[0])
  fi
}
g() {
  local fuzzer=$1
}
fuzzer=$1
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["fuzzer", "fuzzer"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_string_appends_after_scalar_reassignments() {
        let source = "\
#!/bin/bash
exts=(txt pdf)
exts=\"${exts[*]}\"
exts+=\" ${exts^^}\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["exts"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_later_assignments_after_bare_declare_resets() {
        let source = "\
#!/bin/bash
exts=(txt pdf)
f() {
  declare exts
}
g() {
  local exts=base
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_array_style_references_without_prior_array_bindings() {
        let source = "\
#!/bin/bash
echo \"${exts[@]}\"
exts=base
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
