use crate::{Checker, Rule, Violation, static_word_text};

pub struct SuWithoutFlag;

impl Violation for SuWithoutFlag {
    fn rule() -> Rule {
        Rule::SuWithoutFlag
    }

    fn message(&self) -> String {
        "use `su -l` or `su -c` when switching users".to_owned()
    }
}

pub fn su_without_flag(checker: &mut Checker) {
    let spans = checker
        .facts()
        .commands()
        .iter()
        .filter(|fact| fact.effective_name_is("su") && fact.wrappers().is_empty())
        .filter(|fact| !su_has_login_or_command_flag(fact, checker.source()))
        .filter_map(|fact| fact.body_name_word().map(|word| word.span))
        .collect::<Vec<_>>();

    checker.report_all(spans, || SuWithoutFlag);
}

fn su_has_login_or_command_flag(fact: &crate::CommandFact<'_>, source: &str) -> bool {
    let args = fact.body_args();
    let mut pending_option_arg = false;
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if pending_option_arg {
                pending_option_arg = false;
                index += 1;
                continue;
            }
            return false;
        };

        if pending_option_arg {
            pending_option_arg = false;
            index += 1;
            continue;
        }

        match text.as_str() {
            "-" | "-l" | "--login" => return true,
            "--" => break,
            "--command" => return args.get(index + 1).is_some(),
            _ if text.starts_with("--command=") => return text.len() > "--command=".len(),
            _ => {}
        }

        if !text.starts_with('-') {
            break;
        }

        let mut flags = text[1..].chars().peekable();
        while let Some(flag) = flags.next() {
            match flag {
                'l' => return true,
                'c' => return flags.peek().is_some() || args.get(index + 1).is_some(),
                flag if su_short_option_takes_argument(flag) => {
                    if flags.peek().is_none() {
                        pending_option_arg = true;
                    }
                    break;
                }
                _ => {}
            }
        }

        index += 1;
    }

    false
}

fn su_short_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'C' | 'g' | 'G' | 's' | 'w')
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_plain_su_invocations_without_login_or_command_flags() {
        let source = "\
#!/bin/bash
su librenms
su -c
su --command
su librenms -c id
su alice -
su alice -- -
command su librenms
sudo su librenms
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SuWithoutFlag));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["su", "su", "su", "su", "su", "su"]
        );
    }

    #[test]
    fn ignores_login_and_command_forms() {
        let source = "\
#!/bin/bash
su -
su -l
su -l root
su --login root
su -c id root
su -cid root
su --command id root
su - root
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::SuWithoutFlag));

        assert!(diagnostics.is_empty());
    }
}
