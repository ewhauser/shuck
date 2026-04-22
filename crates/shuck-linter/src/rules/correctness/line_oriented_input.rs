use crate::{Checker, Rule, Violation};

pub struct LineOrientedInput;

impl Violation for LineOrientedInput {
    fn rule() -> Rule {
        Rule::LineOrientedInput
    }

    fn message(&self) -> String {
        "iterating over command output in a `for` loop splits lines on whitespace".to_owned()
    }
}

pub fn line_oriented_input(checker: &mut Checker) {
    let spans = checker
        .facts()
        .for_headers()
        .iter()
        .filter(|header| header.words().len() == 1)
        .flat_map(|header| header.words().iter())
        .filter(|word| word.contains_line_oriented_substitution())
        .map(|word| word.span())
        .collect::<Vec<_>>();

    checker.report_all(spans, || LineOrientedInput);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_plain_line_oriented_substitutions() {
        let source = "\
for line in $(cat input.txt); do :; done
for line in $(grep foo input.txt); do :; done
for line in $(awk '{print $1}' input.txt | sort); do :; done
for line in $(command sed -n p input.txt); do :; done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LineOrientedInput));

        assert_eq!(diagnostics.len(), 4);
    }

    #[test]
    fn ignores_safe_generators_dynamic_commands_and_mixed_pipelines() {
        let source = "\
helper() { printf '%s\\n' a b; }
cmd=printf
for line in $(printf '%s\\n' alpha beta); do :; done
for line in $(echo alpha beta | rev); do :; done
for line in $(compgen -W 'a b'); do :; done
for line in $(helper); do :; done
for line in $($cmd '%s\\n' alpha beta); do :; done
for line in $(cat input.txt | head -n1); do :; done
for line in $(printf '%s\\n' alpha beta | sed -n p); do :; done
for line in $(find . -type f); do :; done
for line in $(ls | sed -n p); do :; done
for line in $( (grep foo input.txt | cut -d: -f1) ); do :; done
for line in literal $(cat input.txt); do :; done
for line in $(cat input.txt) literal; do :; done
for line in $(find . -type f -exec grep -Pl '\\r$' {} \\;); do :; done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LineOrientedInput));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_ssh_completion_include_loop_shape() {
        let source = "\
for fl in \"$HOME/.ssh/config\" \
  $(grep \"^\\\\s*Include\" \"$HOME/.ssh/config\" \
    | awk '{for (i=2; i<=NF; i++) print $i}' \
    | sed -Ee \"s|^([^/~])|$HOME/.ssh/\\\\1|\" -e \"s|^~/|$HOME/|\"); do
  :
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LineOrientedInput));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn ignores_find_exec_grep_pl_loop_shape() {
        let source = "\
for FILE in $(find . -type f -exec grep -Pl '\\r$' {} \\;); do
  :
done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::LineOrientedInput));

        assert!(diagnostics.is_empty());
    }
}
