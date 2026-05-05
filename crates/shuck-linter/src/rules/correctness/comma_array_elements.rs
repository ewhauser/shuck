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
    checker.report_fact_slice_dedup(
        |facts| facts.comma_array_assignment_spans(),
        || CommaArrayElements,
    );
}

#[cfg(test)]
mod tests {
    use crate::test::test_snippet;
    use crate::{LinterSettings, Rule, ShellDialect};

    #[test]
    fn reports_comma_separated_array_literals() {
        let source = "\
#!/bin/bash
a=(alpha,beta)
b=(alpha, beta)
c=([k]=v, [q]=w)
d+=(x,y)
e=(foo,{x,y},bar)
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
                "(foo,{x,y},bar)"
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
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CommaArrayElements));

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh_option_map_values() {
        let source = "\
#!/bin/zsh
local -A opts
opts=(
  -q       opt_-q,--quiet:\"update:[quiet mode] *:[less output]\"
  --quiet  opt_-q,--quiet
)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommaArrayElements).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn ignores_zsh_subscript_ranges_and_glob_qualifier_commas() {
        let source = "\
#!/bin/zsh
spinner=(a b c)
spinner=($spinner[2,-1] $spinner[1])
reply+=($_p9k__display_v[k,k+1])
tmp=( ${expanded_path}*(N-*,N-/) )
files=( **/*(.om[1,3]) *.log(#q.om[1,3]) )
parents=( ${(@)${:-{$#parts..1}}/(#m)*/$parent${(pj./.)parts[1,MATCH]}} )
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommaArrayElements).with_shell(ShellDialect::Zsh),
        );

        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn reports_non_zsh_commas_that_resemble_zsh_subscripts_and_qualifiers() {
        let source = "\
#!/bin/bash
spinner=($spinner[2,-1])
files=(**/*(.om[1,3]))
";
        let diagnostics = test_snippet(source, &LinterSettings::for_rule(Rule::CommaArrayElements));

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["($spinner[2,-1])", "(**/*(.om[1,3]))"]
        );
    }

    #[test]
    fn reports_comma_separated_array_literals_in_zsh() {
        let source = "\
#!/bin/zsh
parts=(alpha,beta)
values=(opt_-q,--quiet)
typeset -A opts
unset opts
opts=(opt_-r,--reset)
";
        let diagnostics = test_snippet(
            source,
            &LinterSettings::for_rule(Rule::CommaArrayElements).with_shell(ShellDialect::Zsh),
        );

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.span.slice(source))
                .collect::<Vec<_>>(),
            vec!["(alpha,beta)", "(opt_-q,--quiet)", "(opt_-r,--reset)"]
        );
    }
}
