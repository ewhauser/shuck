use compact_str::CompactString;
use rustc_hash::{FxHashMap, FxHashSet};
use shuck_ast::Name;
use shuck_semantic::{
    Binding, BindingAttributes, BindingKind, DeclarationBuiltin, DeclarationOperand, ReferenceKind,
    ScopeId,
};

use crate::{Checker, Diagnostic, Rule, ShellDialect, Violation};

pub struct ImplicitGlobalInFunction {
    pub name: CompactString,
}

impl Violation for ImplicitGlobalInFunction {
    fn rule() -> Rule {
        Rule::ImplicitGlobalInFunction
    }

    fn message(&self) -> String {
        format!(
            "assignment to `{}` inside a function is not declared local",
            self.name
        )
    }
}

pub fn implicit_global_in_function(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Bash {
        return;
    }

    let semantic = checker.semantic();
    let analysis = checker.semantic_analysis();
    let options = &checker.rule_options().c158;
    let documented_globals = documented_global_names(
        semantic,
        options.treat_readonly_as_documented,
        options.treat_export_as_intentional,
    );
    let local_declarations = function_local_declarations(semantic);
    let diagnostics = semantic
        .bindings()
        .iter()
        .filter(|binding| binding_can_mutate_function_global(binding))
        .filter(|binding| !documented_globals.contains(&binding.name))
        .filter(|binding| !analysis.scope_runs_in_transient_context(binding.scope))
        .filter_map(|binding| {
            let function_scope = semantic.enclosing_function_scope(binding.scope)?;
            let has_local_declaration = local_declarations
                .get(&(function_scope, binding.name.clone()))
                .is_some_and(|offsets| {
                    offsets
                        .iter()
                        .any(|offset| *offset <= binding.span.start.offset)
                });
            (!has_local_declaration).then(|| {
                Diagnostic::new(
                    ImplicitGlobalInFunction {
                        name: binding.name.as_str().into(),
                    },
                    binding.span,
                )
            })
        })
        .collect::<Vec<_>>();

    for diagnostic in diagnostics {
        checker.report_diagnostic_dedup(diagnostic);
    }
}

fn documented_global_names(
    semantic: &shuck_semantic::SemanticModel,
    treat_readonly_as_documented: bool,
    treat_export_as_intentional: bool,
) -> FxHashSet<Name> {
    let mut documented = semantic
        .bindings()
        .iter()
        .filter(|binding| binding_is_top_level_declaration_site(semantic, binding))
        .filter(|binding| {
            (treat_readonly_as_documented && binding_documents_readonly(binding))
                || (treat_export_as_intentional && binding_documents_export(binding))
        })
        .map(|binding| binding.name.clone())
        .collect::<FxHashSet<_>>();

    let documented_attribute_names = semantic
        .bindings()
        .iter()
        .filter(|binding| binding_is_file_scoped(binding, semantic))
        .filter(|binding| {
            (treat_readonly_as_documented && binding_documents_readonly(binding))
                || (treat_export_as_intentional && binding_documents_export(binding))
        })
        .map(|binding| binding.name.clone())
        .collect::<FxHashSet<_>>();

    documented.extend(
        semantic
            .references()
            .iter()
            .filter(|reference| reference.kind == ReferenceKind::DeclarationName)
            .filter(|reference| reference_is_top_level(semantic, reference))
            .filter(|reference| documented_attribute_names.contains(&reference.name))
            .filter(|reference| {
                declaration_reference_documents_global(
                    semantic,
                    reference,
                    treat_readonly_as_documented,
                    treat_export_as_intentional,
                )
            })
            .map(|reference| reference.name.clone()),
    );

    documented
}

fn declaration_reference_documents_global(
    semantic: &shuck_semantic::SemanticModel,
    reference: &shuck_semantic::Reference,
    treat_readonly_as_documented: bool,
    treat_export_as_intentional: bool,
) -> bool {
    semantic.declarations().iter().any(|declaration| {
        declaration_covers_reference(declaration, reference)
            && ((treat_readonly_as_documented && declaration_documents_readonly(declaration))
                || (treat_export_as_intentional && declaration_documents_export(declaration)))
    })
}

fn declaration_covers_reference(
    declaration: &shuck_semantic::Declaration,
    reference: &shuck_semantic::Reference,
) -> bool {
    declaration.operands.iter().any(|operand| match operand {
        DeclarationOperand::Name { name, span } => {
            name == &reference.name && *span == reference.span
        }
        DeclarationOperand::Assignment {
            name_span, name, ..
        } => name == &reference.name && *name_span == reference.span,
        DeclarationOperand::Flag { .. } | DeclarationOperand::DynamicWord { .. } => false,
    })
}

fn declaration_documents_readonly(declaration: &shuck_semantic::Declaration) -> bool {
    matches!(declaration.builtin, DeclarationBuiltin::Readonly)
        || declaration_has_flag(declaration, 'r')
}

fn declaration_documents_export(declaration: &shuck_semantic::Declaration) -> bool {
    matches!(declaration.builtin, DeclarationBuiltin::Export)
        || declaration_has_flag(declaration, 'x')
}

fn declaration_has_flag(declaration: &shuck_semantic::Declaration, flag: char) -> bool {
    declaration.operands.iter().any(|operand| {
        matches!(operand, DeclarationOperand::Flag { flag: candidate, .. } if *candidate == flag)
    })
}

fn binding_is_top_level_declaration_site(
    semantic: &shuck_semantic::SemanticModel,
    binding: &Binding,
) -> bool {
    binding_is_file_scoped(binding, semantic) && matches!(binding.kind, BindingKind::Declaration(_))
}

fn binding_is_file_scoped(binding: &Binding, semantic: &shuck_semantic::SemanticModel) -> bool {
    matches!(
        semantic.scope_kind(binding.scope),
        shuck_semantic::ScopeKind::File
    ) && matches!(
        semantic.scope_kind(semantic.scope_at(binding.span.start.offset)),
        shuck_semantic::ScopeKind::File
    )
}

fn reference_is_top_level(
    semantic: &shuck_semantic::SemanticModel,
    reference: &shuck_semantic::Reference,
) -> bool {
    matches!(
        semantic.scope_kind(reference.scope),
        shuck_semantic::ScopeKind::File
    ) && matches!(
        semantic.scope_kind(semantic.scope_at(reference.span.start.offset)),
        shuck_semantic::ScopeKind::File
    )
}

fn binding_documents_readonly(binding: &Binding) -> bool {
    binding.attributes.contains(BindingAttributes::READONLY)
        || matches!(
            binding.kind,
            BindingKind::Declaration(DeclarationBuiltin::Readonly)
        )
}

fn binding_documents_export(binding: &Binding) -> bool {
    binding.attributes.contains(BindingAttributes::EXPORTED)
        || matches!(
            binding.kind,
            BindingKind::Declaration(DeclarationBuiltin::Export)
        )
}

fn function_local_declarations(
    semantic: &shuck_semantic::SemanticModel,
) -> FxHashMap<(ScopeId, Name), Vec<usize>> {
    let mut declarations = FxHashMap::<(ScopeId, Name), Vec<usize>>::default();
    for binding in semantic.bindings() {
        if !binding_declares_function_local(binding) {
            continue;
        }
        if let Some(function_scope) = semantic.enclosing_function_scope(binding.scope)
            && binding.scope == function_scope
        {
            declarations
                .entry((function_scope, binding.name.clone()))
                .or_default()
                .push(binding.span.start.offset);
        }
    }

    declarations
}

fn binding_declares_function_local(binding: &Binding) -> bool {
    binding.attributes.contains(BindingAttributes::LOCAL)
        || matches!(
            binding.kind,
            BindingKind::Declaration(
                DeclarationBuiltin::Declare
                    | DeclarationBuiltin::Local
                    | DeclarationBuiltin::Readonly
                    | DeclarationBuiltin::Typeset
            ) | BindingKind::Nameref
        )
}

fn binding_can_mutate_function_global(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::Assignment
            | BindingKind::ParameterDefaultAssignment
            | BindingKind::AppendAssignment
            | BindingKind::ArrayAssignment
            | BindingKind::LoopVariable
            | BindingKind::ReadTarget
            | BindingKind::MapfileTarget
            | BindingKind::PrintfTarget
            | BindingKind::GetoptsTarget
            | BindingKind::ZparseoptsTarget
            | BindingKind::ArithmeticAssignment
    )
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_function_assignments_without_prior_local_declarations() {
        let source = "\
#!/bin/bash
work() {
  item=1
  item+=2
  for loop in a b; do
    :
  done
  read -r line
  printf -v rendered '%s' \"$item\"
  (( total += 1 ))
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["item", "item", "loop", "line", "rendered", "total"]
        );
    }

    #[test]
    fn accepts_assignments_after_function_scope_declarations() {
        let source = "\
#!/bin/bash
work() {
  local item=1
  item=2
  declare path
  path=/tmp
  readonly pinned=1
  pinned=2
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn later_local_declarations_do_not_hide_earlier_global_assignments() {
        let source = "\
#!/bin/bash
work() {
  late=1
  local late
  late=2
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["late"]
        );
    }

    #[test]
    fn treats_documented_global_bindings_as_intentional_by_default() {
        let source = "\
#!/bin/bash
readonly PINNED=1
export SHARED=old
work() {
  PINNED=2
  SHARED=new
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn treats_top_level_name_only_export_and_readonly_as_documenting_existing_globals() {
        let source = "\
#!/bin/bash
PINNED=old
readonly PINNED
SHARED=old
export SHARED
work() {
  PINNED=new
  SHARED=new
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn function_export_does_not_document_existing_file_global() {
        let source = "\
#!/bin/bash
SHARED=old
mark_exported() {
  export SHARED
}
work() {
  SHARED=new
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "SHARED");
        assert_eq!(diagnostics[0].span.start.line, 7);
    }

    #[test]
    fn unrelated_top_level_declare_does_not_document_function_exported_global() {
        let source = "\
#!/bin/bash
SHARED=old
mark_exported() {
  export SHARED
}
declare SHARED
work() {
  SHARED=new
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction),
        );

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "SHARED");
        assert_eq!(diagnostics[0].span.start.line, 8);
    }

    #[test]
    fn function_global_declarations_do_not_document_file_globals() {
        let source = "\
#!/bin/bash
export DOCUMENTED=1
declare_global() {
  declare -gx SHARED=1
}
work() {
  DOCUMENTED=2
  SHARED=2
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["SHARED"]
        );
    }

    #[test]
    fn options_can_report_readonly_and_exported_globals() {
        let source = "\
#!/bin/bash
readonly PINNED=1
export SHARED=old
work() {
  PINNED=2
  SHARED=new
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction)
                .with_c158_treat_readonly_as_documented(false)
                .with_c158_treat_export_as_intentional(false),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["PINNED", "SHARED"]
        );
    }

    #[test]
    fn ignores_other_shells_transient_scopes_and_default_settings() {
        let sh_source = "\
#!/bin/sh
work() {
  item=1
}
";
        let transient_source = "\
#!/bin/bash
work() {
  (item=1)
  echo \"$(nested=1)\"
}
";

        assert!(
            test_snippet(
                sh_source,
                &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction),
            )
            .is_empty()
        );
        assert!(
            test_snippet(
                transient_source,
                &LinterSettings::for_rule(Rule::ImplicitGlobalInFunction),
            )
            .is_empty()
        );
        assert!(
            test_snippet(sh_source, &LinterSettings::default())
                .iter()
                .all(|diagnostic| diagnostic.rule != Rule::ImplicitGlobalInFunction)
        );
    }
}
