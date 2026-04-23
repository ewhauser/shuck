use std::collections::HashMap;

use shuck_ast::Name;
use shuck_semantic::{Binding, BindingAttributes, BindingKind, DeclarationBuiltin};

use crate::{Checker, Rule, ShellDialect, Violation, WrapperKind};

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
    let builtin_array_history = builtin_array_history_events(checker);
    let mut next_builtin_array_history = 0usize;
    let mut bindings = semantic.bindings().iter().collect::<Vec<_>>();
    bindings.sort_by_key(|binding| (binding.span.start.offset, binding.span.end.offset));

    let spans = bindings
        .into_iter()
        .filter_map(|binding| {
            while let Some((offset, name)) = builtin_array_history.get(next_builtin_array_history) {
                if *offset > binding.span.start.offset {
                    break;
                }
                array_history.insert(name.clone(), true);
                next_builtin_array_history += 1;
            }

            let name = binding.name.clone();
            let saw_array_history = array_history
                .get(&name)
                .copied()
                .unwrap_or_else(|| binding_uses_builtin_array_history(checker, binding));

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

fn builtin_array_history_events(checker: &Checker<'_>) -> Vec<(usize, Name)> {
    let mut events = checker
        .facts()
        .commands()
        .iter()
        .filter(|command| command_forces_builtin_resolution(command))
        .flat_map(|command| command_array_history_events(checker, command))
        .collect::<Vec<_>>();
    events.sort_by_key(|(offset, _)| *offset);
    events
}

fn command_array_history_events(
    checker: &Checker<'_>,
    command: &crate::facts::commands::CommandFact<'_>,
) -> Vec<(usize, Name)> {
    if matches!(checker.shell(), ShellDialect::Bash) && command.effective_name_is("read") {
        return command
            .options()
            .read()
            .filter(|_| !command_is_shadowed_function(checker, command))
            .map(|read| {
                read.array_target_name_uses()
                    .iter()
                    .map(|target| {
                        (
                            target.span().start.offset,
                            Name::from(target.key().as_str()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    if matches!(checker.shell(), ShellDialect::Bash)
        && (command.effective_name_is("mapfile") || command.effective_name_is("readarray"))
    {
        return command
            .options()
            .mapfile()
            .filter(|_| !command_is_shadowed_function(checker, command))
            .map(|mapfile| {
                mapfile
                    .target_name_uses()
                    .iter()
                    .map(|target| {
                        (
                            target.span().start.offset,
                            Name::from(target.key().as_str()),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    Vec::new()
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
        BindingKind::MapfileTarget => mapfile_target_is_array_like(checker, binding),
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

fn binding_uses_builtin_array_history(checker: &Checker<'_>, binding: &Binding) -> bool {
    matches!(checker.shell(), ShellDialect::Bash) && matches!(binding.name.as_str(), "MAPFILE")
}

fn read_target_is_array_like(checker: &Checker<'_>, binding: &Binding) -> bool {
    if !matches!(checker.shell(), ShellDialect::Bash) {
        return false;
    }

    binding_command(checker, binding)
        .filter(|command| command.effective_name_is("read"))
        .filter(|command| !command_is_shadowed_function(checker, command))
        .and_then(|command| command.options().read())
        .is_some_and(|read| {
            read.array_target_name_uses()
                .iter()
                .any(|target| target.span() == binding.span)
        })
}

fn mapfile_target_is_array_like(checker: &Checker<'_>, binding: &Binding) -> bool {
    if !matches!(checker.shell(), ShellDialect::Bash) {
        return false;
    }

    let Some(command) = binding_command(checker, binding) else {
        return false;
    };
    if !(command.effective_name_is("mapfile") || command.effective_name_is("readarray")) {
        return false;
    }

    !command_is_shadowed_function(checker, command)
        && command.options().mapfile().is_some_and(|mapfile| {
            mapfile
                .target_name_uses()
                .iter()
                .any(|target| target.span() == binding.span)
        })
}

fn binding_command<'a>(
    checker: &'a Checker<'_>,
    binding: &Binding,
) -> Option<&'a crate::facts::commands::CommandFact<'a>> {
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
}

fn command_is_shadowed_function(
    checker: &Checker<'_>,
    command: &crate::facts::commands::CommandFact<'_>,
) -> bool {
    if command_forces_builtin_resolution(command) {
        return false;
    }

    let Some(name_span) = command.body_word_span() else {
        return false;
    };
    let Some(command_name) = command.effective_or_literal_name() else {
        return false;
    };

    checker
        .semantic()
        .visible_binding(&command_name.into(), name_span)
        .is_some_and(binding_is_function_like)
}

fn command_forces_builtin_resolution(command: &crate::facts::commands::CommandFact<'_>) -> bool {
    command.has_wrapper(WrapperKind::Command) || command.has_wrapper(WrapperKind::Builtin)
}

fn contains_span(outer: shuck_ast::Span, inner: shuck_ast::Span) -> bool {
    outer.start.offset <= inner.start.offset && inner.end.offset <= outer.end.offset
}

fn binding_is_function_like(binding: &Binding) -> bool {
    matches!(binding.kind, BindingKind::FunctionDefinition)
        || binding
            .attributes
            .contains(BindingAttributes::IMPORTED_FUNCTION)
}

fn binding_is_array_like(binding: &Binding) -> bool {
    binding
        .attributes
        .intersects(BindingAttributes::ARRAY | BindingAttributes::ASSOC)
        || binding.kind == BindingKind::ArrayAssignment
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use crate::test::{test_snippet, test_snippet_at_path};
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
    fn reports_scalar_reassignments_after_attached_read_array_targets() {
        let source = "\
#!/bin/bash
f() {
  read -aresolution <<< \"1 2 3\"
  resolution=\"${resolution[0]} x ${resolution[1]} @ ${resolution[2]} fps\"
  read -ar <<< \"4 5 6\"
  r=\"${r[0]} x ${r[1]} @ ${r[2]} fps\"
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
            vec!["resolution", "r"],
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
    fn ignores_mapfile_scalar_assignments_outside_bash() {
        let source = "\
#!/bin/sh
mapfile entries
entries=value
MAPFILE=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_mapfile_targets_from_shadowing_functions() {
        let source = "\
#!/bin/bash
mapfile() {
  :
}
mapfile entries
entries=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_mapfile_callback_names() {
        let source = "\
#!/bin/bash
mapfile -C cb -c 1 lines
cb=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_read_array_history_through_command_wrapper() {
        let source = "\
#!/bin/bash
read() {
  :
}
command read -a entries
entries=value
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
            vec!["entries"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn reports_mapfile_history_through_builtin_wrapper() {
        let source = "\
#!/bin/bash
mapfile() {
  :
}
builtin mapfile lines
lines=value
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
            vec!["lines"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn reports_quoted_wrapper_targets_as_array_history() {
        let source = "\
#!/bin/bash
read() {
  :
}
mapfile() {
  :
}
command read -a \"entries\"
builtin mapfile 'lines'
entries=value
lines=value
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
            vec!["entries", "lines"],
            "{diagnostics:#?}"
        );
    }

    #[test]
    fn ignores_read_scalar_assignments_outside_bash() {
        let source = "\
#!/bin/sh
read -a entries
entries=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_read_targets_from_shadowing_functions() {
        let source = "\
#!/bin/bash
read() {
  :
}
read -a entries
entries=value
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_mapfile_targets_from_imported_shadowing_functions() {
        let temp = tempdir().unwrap();
        let main = temp.path().join("main.sh");
        let helper = temp.path().join("helper.sh");
        let source = "\
#!/bin/bash
source ./helper.sh
mapfile entries
entries=value
";

        fs::write(&main, source).unwrap();
        fs::write(&helper, "mapfile() { :; }\n").unwrap();

        let diagnostics = test_snippet_at_path(
            &main,
            source,
            &LinterSettings::for_rule(Rule::ArrayToStringConversion)
                .with_analyzed_paths([main.clone(), helper.clone()]),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
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
