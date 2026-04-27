use shuck_semantic::{Binding, BindingAttributes, BindingKind, DeclarationBuiltin, ScopeKind};

use crate::{Checker, Rule, ShellDialect, Violation};

pub struct LocalTopLevel;

impl Violation for LocalTopLevel {
    fn rule() -> Rule {
        Rule::LocalTopLevel
    }

    fn message(&self) -> String {
        "`local` is only valid inside a function body".to_owned()
    }
}

pub fn local_top_level(checker: &mut Checker) {
    if !matches!(checker.shell(), ShellDialect::Bash | ShellDialect::Dash) {
        return;
    }

    let semantic = checker.semantic();
    let spans = semantic
        .declarations()
        .iter()
        .filter(|declaration| declaration.builtin == DeclarationBuiltin::Local)
        .filter(|declaration| {
            checker.first_parse_error().is_none_or(|(line, column)| {
                declaration.span.start.line < line
                    || (declaration.span.start.line == line
                        && declaration.span.start.column < column)
            })
        })
        .filter_map(|declaration| {
            let binding_scopes = semantic
                .bindings_in_span(declaration.span)
                .filter(|binding| local_binding_belongs_to_declaration_kind(binding))
                .map(|binding| binding.scope)
                .collect::<Vec<_>>();

            if binding_scopes
                .iter()
                .any(|scope| matches!(semantic.scope_kind(*scope), ScopeKind::Function(_)))
            {
                return None;
            }

            if !binding_scopes.is_empty() {
                return Some(declaration.span);
            }

            let scope = semantic.scope_at(declaration.span.start.offset);
            let inside_function = semantic
                .ancestor_scopes(scope)
                .any(|scope| matches!(semantic.scope_kind(scope), ScopeKind::Function(_)));

            (!inside_function).then_some(declaration.span)
        })
        .collect();

    checker.report_all_dedup(spans, || LocalTopLevel);
}

fn local_binding_belongs_to_declaration_kind(binding: &Binding) -> bool {
    matches!(
        binding.kind,
        BindingKind::Declaration(DeclarationBuiltin::Local)
    ) || (matches!(binding.kind, BindingKind::Nameref)
        && binding.attributes.contains(BindingAttributes::LOCAL))
}

#[cfg(test)]
mod tests {
    use shuck_parser::parser::Parser;

    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn ignores_local_inside_function_branches() {
        let source = "\
#!/bin/bash
f() {
  if true; then
    local scoped=1
  fi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LocalTopLevel));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_local_inside_function_loops() {
        let source = "\
#!/bin/bash
f() {
  local -a items=(one two)
  for (( i=0; i<${#items[@]}; i++ )); do
    local item=${items[i]}
    printf '%s\\n' \"$item\"
  done
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LocalTopLevel));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn ignores_redeclared_locals_inside_functions() {
        let source = "\
#!/bin/bash
f() {
  local item=first
  if true; then
    local item=second
    printf '%s\\n' \"$item\"
  fi
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LocalTopLevel));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }

    #[test]
    fn reports_top_level_local_nameref() {
        let source = "\
#!/bin/bash
local -n ref=target
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LocalTopLevel));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].rule, Rule::LocalTopLevel);
        assert!(
            diagnostics[0]
                .span
                .slice(source)
                .starts_with("local -n ref=target")
        );
    }

    #[test]
    fn ignores_locals_after_recovered_parse_errors() {
        let source = "\
#!/bin/bash
f() {
  if true; then
    :
  else
}

g() {
  local recovered=1
}
";
        let parse_result = Parser::new(source).parse();
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LocalTopLevel));

        assert!(parse_result.is_err(), "expected recovered parse");
        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
