use shuck_semantic::ScopeKind;

use crate::{Checker, DeclarationKind, Rule, Violation};

pub struct ExportCommandSubstitution {
    pub name: String,
}

impl Violation for ExportCommandSubstitution {
    fn rule() -> Rule {
        Rule::ExportCommandSubstitution
    }

    fn message(&self) -> String {
        format!("assign command output before declaring `{}`", self.name)
    }
}

pub fn export_command_substitution(checker: &mut Checker) {
    let mut findings = Vec::new();

    for fact in checker.facts().structural_commands() {
        for probe in fact.declaration_assignment_probes() {
            if !should_report_s010_declaration(
                checker,
                probe.kind(),
                probe.readonly_flag(),
                fact.span(),
            ) {
                continue;
            }

            if probe.has_command_substitution() {
                findings.push((probe.target_name().to_owned(), probe.target_name_span()));
            }
        }
    }

    for (name, span) in findings {
        checker.report_dedup(ExportCommandSubstitution { name }, span);
    }
}

fn matches_s010_declaration_kind(kind: &DeclarationKind) -> bool {
    matches!(
        kind,
        DeclarationKind::Export
            | DeclarationKind::Local
            | DeclarationKind::Declare
            | DeclarationKind::Typeset
    ) || matches!(kind, DeclarationKind::Other(name) if name == "readonly")
}

fn should_report_s010_declaration(
    checker: &Checker,
    kind: &DeclarationKind,
    readonly_flag: bool,
    span: shuck_ast::Span,
) -> bool {
    if !matches_s010_declaration_kind(kind) {
        return false;
    }

    if !readonly_flag || matches!(kind, DeclarationKind::Export) {
        return true;
    }

    if matches!(kind, DeclarationKind::Other(name) if name == "readonly") {
        return true;
    }

    let scope = checker.semantic().scope_at(span.start.offset);
    let inside_function = checker
        .semantic()
        .ancestor_scopes(scope)
        .any(|scope| matches!(checker.semantic().scope_kind(scope), ScopeKind::Function(_)));

    !inside_function
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn anchors_on_declaration_assignment_names() {
        let source = "\
#!/bin/bash
export greeting=$(printf '%s\\n' hi)
demo() {
  local temp=\"$(date)\"
  readonly keep_me=$(date)
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["greeting", "temp", "keep_me"]
        );
    }

    #[test]
    fn ignores_function_local_readonly_modifier_declaration_assignments() {
        let source = "\
#!/bin/bash
demo() {
  local -r temp=\"$(date)\"
  declare -r other=$(date)
  typeset -r home=$(pwd)
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_top_level_readonly_declaration_assignments() {
        let source = "\
#!/bin/bash
readonly temp=\"$(mktemp)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["temp"]
        );
    }

    #[test]
    fn reports_top_level_readonly_declare_variants() {
        let source = "\
#!/bin/bash
declare -r archive_dir_create=\"$(mktemp -dt archive_dir_create.XXXXXX)\"
typeset -r n_version=\"$(./bin/n --version)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["archive_dir_create", "n_version"]
        );
    }

    #[test]
    fn reports_top_level_readonly_variants_from_corpus() {
        let source = "\
#!/bin/bash
readonly archive_dir_create=\"$(mktemp -dt archive_dir_create.XXXXXX)\"
readonly n_version=\"$(./bin/n --version)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["archive_dir_create", "n_version"]
        );
    }

    #[test]
    fn reports_escaped_declaration_builtins_with_command_substitution() {
        let source = "\
#!/bin/bash
export_path() {
  \\local local_name=\"$(date)\"
  \\declare declared_name=$(pwd)
  \\typeset typed_name=\"$(mktemp)\"
  \\readonly kept_name=$(date)
}
\\export exported_name=\"$(printf '%s' hi)\"
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "local_name",
                "declared_name",
                "typed_name",
                "kept_name",
                "exported_name",
            ]
        );
    }

    #[test]
    fn ignores_escaped_function_local_readonly_modifier_declarations() {
        let source = "\
#!/bin/bash
demo() {
  \\local -r temp=\"$(date)\"
  \\declare -r other=$(date)
  \\typeset -r home=$(pwd)
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_command_substitutions_in_escaped_assignment_targets() {
        let source = "\
#!/bin/bash
demo() {
  \\local arr[$(date)]=1
  \\declare map[${key:-$(printf '%s' fallback)}]=literal
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_escaped_declarations_with_nested_subscript_brackets() {
        let source = "\
#!/bin/bash
\\declare name[${x:-]}]=$(date)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["name"]
        );
    }

    #[test]
    fn reports_escaped_declarations_with_literal_braces_inside_command_subscripts() {
        let source = "\
#!/bin/bash
\\declare arr[$(echo {)]=$(date)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["arr"]
        );
    }

    #[test]
    fn reports_escaped_declarations_with_utf8_escaped_subscript_chars() {
        let source = "\
#!/bin/bash
\\declare arr[\\\u{00e9}]=$(date)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["arr"]
        );
    }

    #[test]
    fn reports_escaped_declarations_with_nested_parameter_subscript_commands() {
        let source = "\
#!/bin/bash
\\declare arr[${k:-$(printf '}')}]=$(date)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["arr"]
        );
    }

    #[test]
    fn reports_escaped_declarations_with_parameter_expansion_parens_inside_command_subscripts() {
        let source = "\
#!/bin/bash
shopt -s extglob
\\declare arr[$(printf %s ${x//@(a)/b})]=$(date)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::ExportCommandSubstitution),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["arr"]
        );
    }
}
