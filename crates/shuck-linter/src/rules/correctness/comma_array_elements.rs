use crate::{Checker, Rule, Violation};

pub struct CommaArrayElements;

impl Violation for CommaArrayElements {
    fn rule() -> Rule {
        Rule::CommaArrayElements
    }

    fn message(&self) -> String {
        "separate array literal elements with whitespace, not commas".to_owned()
    }
}

pub fn comma_array_elements(checker: &mut Checker) {
    checker.report_all_dedup(
        checker.facts().comma_array_assignment_spans().to_vec(),
        || CommaArrayElements,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule};

    #[test]
    fn reports_comma_separated_array_literals() {
        let source = "\
#!/bin/bash
a=(alpha,beta)
b=(alpha, beta)
c=([k]=v, [q]=w)
d+=(x,y)
e=(foo,{x,y},bar)
f=(\\$((1,2)))
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CommaArrayElements));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec![
                "(alpha,beta)",
                "(alpha, beta)",
                "([k]=v, [q]=w)",
                "(x,y)",
                "(foo,{x,y},bar)",
                "(\\$((1,2)))"
            ]
        );
    }

    #[test]
    fn ignores_quoted_and_brace_expansion_commas() {
        let source = "\
#!/bin/bash
a=(\"alpha,beta\")
b=('alpha,beta')
c=({x,y})
d=($(printf 'x,y'))
e=({$XDG_CONFIG_HOME,$HOME}/{alacritty,}/{.,}alacritty.ym?)
f=(${versions/,/ })
g=(${token//,/ })
h=(\"x\\\",y\")
i=($((1,2)))
j=(${x/\\\"/a,b})
k=(x\\\\\",y\")
l=($(printf %s ${x//foo/)},1))
m=(<(printf %s 1,2))
n=(>(printf %s 3,4))
o=(${x/a,b/{})
p=($'a\\'b,c')
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CommaArrayElements));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_multiline_command_substitution_scanner_edge_cases() {
        let source = "\
#!/bin/bash
a=($(printf '((' # comment with )
printf %s 1,2
))
b=($( ((x<<2))
printf %s 3,4
))
c=($( (case $kind in
a) printf %s 5,6 ;;
esac
) ))
d=(\"$( (#comment with )
printf %s 7,8
) )\")
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CommaArrayElements));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
