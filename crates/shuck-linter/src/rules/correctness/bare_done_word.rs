use crate::{Checker, Rule, ShellDialect, Violation};

pub struct BareDoneWord;

impl Violation for BareDoneWord {
    fn rule() -> Rule {
        Rule::BareDoneWord
    }

    fn message(&self) -> String {
        "put a separator before this loop keyword, or quote it as literal text".to_owned()
    }
}

pub fn bare_done_word(checker: &mut Checker) {
    if checker.shell() == ShellDialect::Zsh {
        return;
    }

    let spans = checker.facts().words().bare_done_word_spans().to_vec();
    checker.report_all_dedup(spans, || BareDoneWord);
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_bare_done_words_in_command_like_positions() {
        let source = "\
#!/bin/sh
printf '%s\\n' done
printf '%s\\n' do
command done
x=done printf '%s\\n' ok
export x=done
rvm 3.1.3 do rvm gemdir
: do
echo hi > done
trap done EXIT
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BareDoneWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
                .collect::<Vec<_>>(),
            vec![
                (2, 15),
                (3, 15),
                (4, 9),
                (5, 3),
                (6, 10),
                (7, 11),
                (8, 3),
                (9, 11),
                (10, 6)
            ]
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.span.start == diagnostic.span.end)
        );
    }

    #[test]
    fn reports_bare_done_words_in_tests_and_lists() {
        let source = "\
#!/bin/sh
[ \"$state\" = done ]
[[ $state == done ]]
[[ $state == do ]]
case done in ok) :;; esac
for value in done; do :; done
select value in done; do :; done
for value in do; do :; done
select value in do; do :; done
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BareDoneWord));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.span.start.line, diagnostic.span.start.column))
                .collect::<Vec<_>>(),
            vec![(2, 14), (3, 14), (4, 14), (5, 6), (6, 14), (7, 17)]
        );
    }

    #[test]
    fn ignores_literal_done_when_the_syntax_is_not_command_like() {
        let source = "\
#!/bin/sh
echo \"done\" 'done' d\"on\"e
echo \"do\" 'do' d\"o\"
echo done.x do.x done#suffix do#suffix
./do
case \"$state\" in done | do) :;; esac
[[ $state =~ done ]]
[[ $state =~ do ]]
echo ${state:-done}
echo ${state:-do}
echo ${state%done}
echo ${state%do}
array[done]=value
array[do]=value
echo ${array[done]}
echo ${array[do]}
for value in do; do :; done
select value in do; do :; done
cat <<do
do
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::BareDoneWord));

        assert!(diagnostics.is_empty(), "diagnostics: {diagnostics:?}");
    }
}
