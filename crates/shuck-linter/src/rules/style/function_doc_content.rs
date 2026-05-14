use crate::{Checker, Rule, Violation};

pub struct FunctionDocContent {
    name: String,
    missing_sections: Vec<DocSection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocSection {
    Globals,
    Arguments,
    Outputs,
    Returns,
}

impl DocSection {
    fn label(self) -> &'static str {
        match self {
            Self::Globals => "Globals",
            Self::Arguments => "Arguments",
            Self::Outputs => "Outputs",
            Self::Returns => "Returns",
        }
    }
}

impl Violation for FunctionDocContent {
    fn rule() -> Rule {
        Rule::FunctionDocContent
    }

    fn message(&self) -> String {
        format!(
            "document function `{}` with missing sections: {}",
            self.name,
            self.missing_sections
                .iter()
                .map(|section| section.label())
                .collect::<Vec<_>>()
                .join(", ")
        )
    }
}

pub fn function_doc_content(checker: &mut Checker) {
    let options = checker.rule_options().s084.clone();
    let violations = checker
        .facts()
        .command_facts()
        .function_doc_content()
        .iter()
        .filter(|fact| fact.has_leading_comment())
        .filter_map(|fact| {
            let missing_sections = missing_doc_sections(fact, &options);
            (!missing_sections.is_empty()).then(|| {
                (
                    fact.name_span(),
                    FunctionDocContent {
                        name: fact.name().as_str().to_owned(),
                        missing_sections,
                    },
                )
            })
        })
        .collect::<Vec<_>>();

    for (span, violation) in violations {
        checker.report(violation, span);
    }
}

fn missing_doc_sections(
    fact: &crate::facts::FunctionDocContentFact,
    options: &crate::S084RuleOptions,
) -> Vec<DocSection> {
    let documented = fact.documented_sections();
    let mut missing = Vec::new();

    if options.require_globals && fact.uses_global_variables() && !documented.has_globals() {
        missing.push(DocSection::Globals);
    }
    if options.require_arguments && fact.uses_positional_parameters() && !documented.has_arguments()
    {
        missing.push(DocSection::Arguments);
    }
    if options.require_outputs && fact.writes_stdout() && !documented.has_outputs() {
        missing.push(DocSection::Outputs);
    }
    if options.require_returns && fact.has_explicit_return() && !documented.has_returns() {
        missing.push(DocSection::Returns);
    }

    missing
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    fn diagnostics(source: &str) -> Vec<crate::Diagnostic> {
        test_snippet(source, &LinterSettings::for_rule(Rule::FunctionDocContent))
    }

    #[test]
    fn reports_missing_sections_for_observed_body_behavior() {
        let source = "\
#!/bin/bash
# Builds an absolute output path.
build_path() {
  local suffix=$1
  echo \"${BASE_DIR}/${suffix}\"
  return 0
}
";
        let diagnostics = diagnostics(source);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "build_path");
        assert!(diagnostics[0].message.contains("Globals"));
        assert!(diagnostics[0].message.contains("Arguments"));
        assert!(diagnostics[0].message.contains("Outputs"));
        assert!(diagnostics[0].message.contains("Returns"));
    }

    #[test]
    fn accepts_complete_function_comment_sections() {
        let source = "\
#!/bin/bash
# Builds an absolute output path by joining BASE_DIR with a suffix.
#
# Globals:
#   BASE_DIR
# Arguments:
#   $1 - The path suffix to append.
# Outputs:
#   Writes the constructed path to stdout.
# Returns:
#   0 always.
build_path() {
  local suffix=$1
  echo \"${BASE_DIR}/${suffix}\"
  return 0
}
";
        assert!(diagnostics(source).is_empty());
    }

    #[test]
    fn skips_functions_without_leading_doc_comment() {
        let source = "\
#!/bin/bash
build_path() {
  echo \"${BASE_DIR}/$1\"
  return 0
}
";
        assert!(diagnostics(source).is_empty());
    }

    #[test]
    fn ignores_local_variables_and_non_stdout_prints() {
        let source = "\
#!/bin/bash
# Prepares local state.
prepare() {
  local target
  printf -v target '%s' ok
  echo hidden >out.txt
}
";
        assert!(diagnostics(source).is_empty());
    }

    #[test]
    fn attached_printf_v_option_assigns_without_stdout_output() {
        let source = "\
#!/bin/bash
# Updates shared state.
set_state() {
  printf -vSTATE '%s' ok
}
";
        let diagnostics = diagnostics(source);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "set_state");
        assert!(diagnostics[0].message.contains("Globals"));
        assert!(!diagnostics[0].message.contains("Outputs"));
    }

    #[test]
    fn printf_option_end_keeps_later_v_text_as_stdout_output() {
        let source = "\
#!/bin/bash
# Prints the state marker.
emit_state() {
  printf -- -vSTATE
}
";
        let diagnostics = diagnostics(source);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "emit_state");
        assert!(!diagnostics[0].message.contains("Globals"));
        assert!(diagnostics[0].message.contains("Outputs"));
    }

    #[test]
    fn path_qualified_printf_counts_as_stdout_output() {
        let source = "\
#!/bin/bash
# Prints a status line.
emit_status() {
  /usr/bin/printf '%s\\n' ready
}
";
        let diagnostics = diagnostics(source);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "emit_status");
        assert!(diagnostics[0].message.contains("Outputs"));
    }

    #[test]
    fn reports_global_writes_without_globals_section() {
        let source = "\
#!/bin/bash
# Updates the shared state.
set_state() {
  STATE=ready
}
";
        let diagnostics = diagnostics(source);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].span.slice(source), "set_state");
        assert!(diagnostics[0].message.contains("Globals"));
    }

    #[test]
    fn ignores_returns_nested_in_command_substitutions() {
        let source = "\
#!/bin/bash
# Prepares local state.
prepare() {
  local value
  value=$(return 7)
}
";
        assert!(diagnostics(source).is_empty());
    }

    #[test]
    fn section_options_can_be_disabled_independently() {
        let source = "\
#!/bin/bash
# Builds an absolute output path.
#
# Globals:
#   BASE_DIR
# Arguments:
#   $1 - The path suffix.
build_path() {
  local suffix=$1
  echo \"${BASE_DIR}/${suffix}\"
  return 0
}
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::FunctionDocContent)
                .with_s084_require_outputs(false)
                .with_s084_require_returns(false),
        );

        assert!(diagnostics.is_empty());
    }
}
