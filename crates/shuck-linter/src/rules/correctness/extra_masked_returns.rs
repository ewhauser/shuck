use crate::{Checker, DeclarationKind, Rule, ShellDialect, Violation};

pub struct ExtraMaskedReturns;

impl Violation for ExtraMaskedReturns {
    fn rule() -> Rule {
        Rule::ExtraMaskedReturns
    }

    fn message(&self) -> String {
        "run this command separately so its exit status is visible".to_owned()
    }
}

pub fn extra_masked_returns(checker: &mut Checker) {
    if checker.shell() != ShellDialect::Bash {
        return;
    }

    let mut spans = Vec::new();
    for fact in checker
        .facts()
        .command_facts()
        .extra_masked_return_declaration_facts()
    {
        if !extra_masking_form_is_enabled(checker, fact.kind(), fact.readonly_flag(), fact.span()) {
            continue;
        }

        spans.extend(fact.masked_return_command_spans().iter().copied());
    }

    checker.report_all_dedup(spans, || ExtraMaskedReturns);
}

fn extra_masking_form_is_enabled(
    checker: &Checker<'_>,
    kind: &DeclarationKind,
    readonly_flag: bool,
    span: shuck_ast::Span,
) -> bool {
    if s010_would_report(checker, kind, readonly_flag, span) {
        return false;
    }

    let forms = &checker.rule_options().c162.treat_as_masking;
    if readonly_flag
        && matches!(kind, DeclarationKind::Declare | DeclarationKind::Local)
        && forms.iter().any(|form| form == "readonly")
    {
        return true;
    }

    readonly_flag
        && matches!(kind, DeclarationKind::Typeset)
        && forms.iter().any(|form| form == "typeset")
}

fn s010_would_report(
    checker: &Checker<'_>,
    kind: &DeclarationKind,
    readonly_flag: bool,
    span: shuck_ast::Span,
) -> bool {
    if !matches!(
        kind,
        DeclarationKind::Export
            | DeclarationKind::Local
            | DeclarationKind::Declare
            | DeclarationKind::Typeset
    ) && !matches!(kind, DeclarationKind::Other(name) if name == "readonly")
    {
        return false;
    }

    if !readonly_flag || matches!(kind, DeclarationKind::Export) {
        return true;
    }

    if matches!(kind, DeclarationKind::Other(name) if name == "readonly") {
        return true;
    }

    let inside_function = checker
        .semantic_analysis()
        .enclosing_function_scope_at(span.start.offset)
        .is_some();

    !inside_function
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_function_local_readonly_local_assignment() {
        let source = "\
#!/bin/bash
demo() {
  local -r result=$(get_value)
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtraMaskedReturns));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "get_value");
    }

    #[test]
    fn reports_function_readonly_declare_assignment_by_default() {
        let source = "\
#!/bin/bash
demo() {
  declare -r result=$(get_value)
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtraMaskedReturns));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "get_value");
    }

    #[test]
    fn ignores_default_masked_return_declarations() {
        let source = "\
#!/bin/bash
demo() {
  local result=$(get_value)
  readonly kept=$(get_value)
  export exported=$(get_value)
  export \"$(dynamic_env)\"
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtraMaskedReturns));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_typeset_readonly_by_default() {
        let source = "\
#!/bin/bash
demo() {
  typeset -r result=$(get_value)
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtraMaskedReturns));

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "get_value");
    }

    #[test]
    fn typeset_readonly_can_be_disabled() {
        let source = "\
#!/bin/bash
demo() {
  typeset -r result=$(get_value)
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExtraMaskedReturns)
                .with_c162_treat_as_masking(["readonly"]),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_each_command_in_enabled_declaration_assignments() {
        let source = "\
#!/bin/bash
demo() {
  local -r result=$(one)$(two)
  local -r nested=$(echo \"$(inner)\")
  local -r printed=$(printf '%s\\n' ok)
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtraMaskedReturns));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["one", "two", "inner"]
        );
    }

    #[test]
    fn ignores_non_declaration_contexts() {
        let source = "\
#!/bin/bash
demo() {
  echo $(get_value)
  printf '%s\\n' $(format_value)
  [[ $(test_value) ]]
  [[ x =~ $(regex_value) ]]
  cat <<< $(stream_value)
  trap \"$(cleanup_value)\" EXIT
  two=$(first)$(second)
  proc=<(process_value)
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtraMaskedReturns));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_declaration_forms_covered_by_s010() {
        let source = "\
#!/bin/bash
demo() {
  local result=$(get_value)
  declare result=$(get_value)
  export result=$(get_value)
  readonly kept=$(get_value)
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtraMaskedReturns));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_escaped_declarations() {
        let source = "\
#!/bin/bash
demo() {
  \\local -r result=$(get_value)
  \\typeset -r strict=$(strict_value)
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtraMaskedReturns));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["get_value", "strict_value"]
        );
    }

    #[test]
    fn skips_command_substitutions_in_declaration_targets() {
        let source = "\
#!/bin/bash
demo() {
  local -r arr[$(target_key)]=literal
}
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::ExtraMaskedReturns));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn option_can_disable_readonly_forms() {
        let source = "\
#!/bin/bash
demo() {
  local -r result=$(get_value)
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExtraMaskedReturns)
                .with_c162_treat_as_masking(["typeset"]),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh() {
        let source = "\
#!/bin/zsh
demo() {
  echo $(get_value)
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExtraMaskedReturns).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
