use super::*;

#[test]
fn literal_brace_spans_include_outer_escaped_parameter_template_edges() {
    let source = "\
#!/bin/sh
eval ac_env_${ac_var}_set=\\${${ac_var}+set}
eval test -n \\\"\\${PG${ev}}\\\" || continue
echo \\${${name}}/\\${fallback}
";

    with_facts(source, None, |_, facts| {
        let positions = facts
            .literal_brace_spans()
            .iter()
            .map(|span| (span.start.line, span.start.column))
            .collect::<Vec<_>>();

        assert_eq!(
            positions,
            vec![
                (2, 29),
                (2, 43),
                (3, 18),
                (3, 26),
                (4, 8),
                (4, 16),
                (4, 20),
                (4, 29),
            ]
        );
    });
}
