use crate::{Checker, Rule, Violation};

pub struct CommandOutputArraySplit;

impl Violation for CommandOutputArraySplit {
    fn rule() -> Rule {
        Rule::CommandOutputArraySplit
    }

    fn message(&self) -> String {
        "avoid splitting command output directly into arrays; use mapfile or read -a".to_owned()
    }
}

pub fn command_output_array_split(checker: &mut Checker) {
    let spans = checker
        .facts()
        .array_assignment_split_word_facts()
        .flat_map(|fact| fact.unquoted_command_substitution_spans().iter().copied())
        .collect::<Vec<_>>();

    checker.report_all_dedup(spans, || CommandOutputArraySplit);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_snippet, test_snippet_at_path};
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_unquoted_command_substitutions_in_array_assignments() {
        let source = "\
#!/bin/bash
arr=($(printf '%s\\n' a b) `printf '%s\\n' c d` prefix$(printf '%s' z)suffix)
declare listed=($(printf '%s\\n' one two))
arr+=($(printf '%s\\n' tail))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommandOutputArraySplit),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "$(printf '%s\\n' a b)",
                "`printf '%s\\n' c d`",
                "$(printf '%s' z)",
                "$(printf '%s\\n' one two)",
                "$(printf '%s\\n' tail)"
            ]
        );
    }

    #[test]
    fn ignores_quoted_and_non_split_array_contexts() {
        let source = "\
#!/bin/bash
arr=(\"$(printf '%s\\n' a b)\" \"`printf '%s\\n' c d`\")
value=$(printf '%s\\n' scalar)
arr=([0]=$(printf '%s\\n' keyed))
declare -A map=([k]=$(printf '%s\\n' assoc))
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommandOutputArraySplit),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn reports_command_output_array_split_in_nb_theme_validation_flow() {
        let source = r#"#!/usr/bin/env bash
set -o noglob
IFS=$'\n\t'

validate_theme() {
  if ! _bat --command-exists
  then
    printf "bat required\n"
  elif [[ -z "${1:-}" ]]
  then
    return 1
  else
    local _theme_list=
    _theme_list=($(_bat --list-themes --color never))

    local __theme=
    for __theme in "${_theme_list[@]}"
    do
      if [[ "$1" == "${__theme:-}" ]]
      then
        return 0
      fi
    done
  fi
}
"#;
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/scripts/xwmx__nb__nb"),
            source,
            &LinterSettings::for_rule(Rule::CommandOutputArraySplit),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(_bat --list-themes --color never)"]
        );
    }

    #[test]
    fn keeps_s018_before_following_extglob_parameter_pattern_condition() {
        let source = r#"#!/usr/bin/env bash
shopt -s extglob
check() {
  local _theme_list=
  _theme_list=($(printf '%s\n' 'Solarized (dark)' base16))
  local __theme=
  for __theme in "${_theme_list[@]}"
  do
    if [[ "${2}" =~ \( ]]
    then
      if [[ "${2}" == "${__theme:-}" ]]
      then
        return 0
      fi
    elif [[ "${2}" =~ ^${__theme%% (*} ]]
    then
      return 0
    fi
  done
}
"#;
        let diagnostics = test_snippet_at_path(
            Path::new("/tmp/scripts/xwmx__nb__nb"),
            source,
            &LinterSettings::for_rule(Rule::CommandOutputArraySplit),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["$(printf '%s\\n' 'Solarized (dark)' base16)"]
        );
    }
}
