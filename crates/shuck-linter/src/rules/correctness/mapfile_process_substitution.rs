use crate::{Checker, Rule, Violation};

pub struct MapfileProcessSubstitution;

impl Violation for MapfileProcessSubstitution {
    fn rule() -> Rule {
        Rule::MapfileProcessSubstitution
    }

    fn message(&self) -> String {
        "`mapfile` reads from a process substitution".to_owned()
    }
}

pub fn mapfile_process_substitution(checker: &mut Checker) {
    // Keep C109 reserved for historical SC2339 compatibility, but stay aligned
    // with current ShellCheck behavior: `mapfile`/`readarray` can safely read
    // from process substitutions because the builtin still runs in the current
    // shell.
    let _ = checker;
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn accepts_process_substitution_inputs() {
        let source = "\
mapfile -t files < <(find . -name '*.pyc')
readarray -t files < <(find . -name '*.log')
mapfile -u 3 -t files 3< <(find . -name '*.tmp')
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MapfileProcessSubstitution),
        );

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn stays_quiet_for_other_inputs() {
        let source = "\
find . -name '*.pyc' | mapfile -t files
mapfile -t files < input.txt
mapfile -t files >(wc -l)
tmp=<(find . -name '*.tmp') mapfile -t files < input.txt
tmp=<(find . -name '*.tmp') readarray -t files < input.txt
mapfile -t files 3< <(find . -name '*.pyc') < input.txt
readarray -u 4 -t files 3< <(find . -name '*.log')
mapfile -u \"$fd\" -t files < <(find . -name '*.pyc')
readarray -u \"${fd}\" -t files 3< <(find . -name '*.log')
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::MapfileProcessSubstitution),
        );

        assert!(diagnostics.is_empty());
    }
}
