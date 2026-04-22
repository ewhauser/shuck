use crate::{Checker, FixAvailability, Rule, Violation};

pub struct SubstWithRedirect;

impl Violation for SubstWithRedirect {
    const FIX_AVAILABILITY: FixAvailability = FixAvailability::None;

    fn rule() -> Rule {
        Rule::SubstWithRedirect
    }

    fn message(&self) -> String {
        "command substitution redirects its output away".to_owned()
    }

    fn fix_title(&self) -> Option<String> {
        None
    }
}

pub fn subst_with_redirect(_checker: &mut Checker) {
    // The pinned ShellCheck oracle currently does not surface SC2255, even for
    // direct reproductions, so keep C057 dormant until there is observable
    // oracle behavior to match.
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::test::{test_path, test_snippet};
    use crate::{LinterSettings, Rule, assert_diagnostics};

    #[test]
    fn stays_quiet_for_rerouted_substitutions_under_current_oracle() {
        let source = "\
opts=$(getopt -o a -- \"$@\" || { usage >&2 && false; })
menu=$(whiptail --menu pick 0 0 0 foo bar 3>&1 1>&2 2>&3)
dialog_out=$(dialog --menu pick 0 0 0 foo bar 3>&1 1>&2 2>&3)
json=$(jq -r . <<< \"$status\" || die >&2)
awk_output=$(awk 'BEGIN { print \"ok\" }' || warn >&2)
choice=$(\"${cmd[@]}\" \"${options[@]}\" 2>&1 >/dev/tty)
out=$(printf quiet >/dev/null; printf loud > out.txt)
out=$(printf hi > out.txt)
out=$(printf hi >&2)
out=$(printf hi > \"$target\")
out=$(printf hi > ${targets[@]})
out=$(printf hi 2>&\"$fd\")
declare arr[$(printf hi > out.txt)]=1
declare -A map=([$(printf bye > \"$target\")]=1)
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstWithRedirect));

        assert!(diagnostics.is_empty());
    }

    #[test]
    fn keeps_fixture_quiet() -> anyhow::Result<()> {
        let (diagnostics, source) = test_path(
            Path::new("correctness").join("C057.sh").as_path(),
            &LinterSettings::for_rule(Rule::SubstWithRedirect),
        )?;

        assert_diagnostics!("C057_C057.sh", diagnostics, &source);
        Ok(())
    }

    #[test]
    fn keeps_direct_redirect_examples_quiet() {
        let source = "#!/bin/sh\nout=$(printf hi > out.txt)\n";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SubstWithRedirect));

        assert!(diagnostics.is_empty());
    }
}
