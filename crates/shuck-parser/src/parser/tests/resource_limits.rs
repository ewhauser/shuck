//! Adversarial nesting and size inputs.
//!
//! Each input here used to overflow the native stack or take exponential time.
//! The tests run on the default test thread stack (2 MiB), so they also check
//! that the parser stays inside a small thread stack in debug builds.

use super::*;

fn parse_bash(input: &str) -> ParseResult {
    Parser::new(input).parse()
}

fn assert_limit_error(output: &ParseResult, needle: &str) {
    assert!(
        output.is_err(),
        "expected a resource-limit error, got {:?}",
        output.status
    );
    assert!(
        output
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(needle)),
        "expected a diagnostic containing {needle:?}, got {:?}",
        output.diagnostics
    );
}

fn nested(open: &str, middle: &str, close: &str, depth: usize) -> String {
    format!("{}{middle}{}\n", open.repeat(depth), close.repeat(depth))
}

#[test]
fn deeply_nested_command_substitutions_report_nesting_error() {
    let output = parse_bash(&format!("echo {}", nested("$(", "x", ")", 400)));
    assert_limit_error(&output, "nesting too deep");
}

#[test]
fn deeply_nested_process_substitutions_report_nesting_error() {
    let output = parse_bash(&format!("cat {}", nested("<(", "x", ")", 400)));
    assert_limit_error(&output, "nesting too deep");
}

#[test]
fn substitution_nesting_counts_toward_the_configured_depth() {
    let input = format!("echo {}", nested("$(echo ", "x", ")", 12));
    let output = Parser::with_limits_and_profile(
        &input,
        8,
        100_000,
        ShellProfile::native(ShellDialect::Bash),
    )
    .parse();
    assert_limit_error(&output, "AST nesting too deep");

    let output = Parser::with_limits_and_profile(
        &input,
        20,
        100_000,
        ShellProfile::native(ShellDialect::Bash),
    )
    .parse();
    assert!(output.is_ok(), "{:?}", output.diagnostics);
}

#[test]
fn nested_parsers_share_the_callers_fuel() {
    let commands = (0..200)
        .map(|index| format!("echo {index}"))
        .collect::<Vec<_>>()
        .join("; ");
    let input = format!("echo \"$( {commands} )\"\n");

    let output = Parser::with_fuel(&input, 100).parse();
    assert_limit_error(&output, "parser fuel exhausted");

    assert!(Parser::new(&input).parse().is_ok());
}

#[test]
fn deeply_nested_zsh_parameter_targets_report_nesting_error() {
    let input = format!("echo {}", nested("${", "a", "}", 3000));
    let output = Parser::with_dialect(&input, ShellDialect::Zsh).parse();
    assert_limit_error(&output, "nesting too deep");
}

#[test]
fn deeply_nested_arithmetic_reports_nesting_error() {
    for input in [
        format!("echo $(({}))\n", nested("(", "1", ")", 240).trim_end()),
        format!("echo $(({}1))\n", "!".repeat(20_000)),
        format!("echo $(({}1))\n", "- ".repeat(20_000)),
        format!("echo $(({}1{}))\n", "1?".repeat(5_000), ":1".repeat(5_000)),
        format!("echo $(({}1))\n", "a=".repeat(10_000)),
        format!("echo $(({}1))\n", "2**".repeat(10_000)),
        format!("(( {}1{} ))\n", "a[".repeat(5_000), "]".repeat(5_000)),
    ] {
        let output = parse_bash(&input);
        assert_limit_error(&output, "nesting too deep");
    }
}

#[test]
fn long_operator_chains_report_length_error() {
    let output = parse_bash(&format!("{}a\n", "a && ".repeat(5_000)));
    assert_limit_error(&output, "command list too long");

    let output = parse_bash(&format!("{}a\n", "a | ".repeat(20_000)));
    assert_limit_error(&output, "pipeline too long");

    let output = parse_bash(&format!("[[ {}a ]]\n", "a && ".repeat(40_000)));
    assert_limit_error(&output, "conditional expression too long");
}

#[test]
fn operator_chains_under_the_limit_parse_cleanly() {
    let output = parse_bash(&format!("{}a\n", "a && ".repeat(999)));
    assert!(output.is_ok(), "{:?}", output.diagnostics);
}

#[test]
fn deeply_nested_conditional_groups_report_nesting_error() {
    let output = parse_bash(&format!("[[ {}a ]]\n", "! ".repeat(3_000)));
    assert_limit_error(&output, "nesting too deep");

    let output = parse_bash(&format!(
        "[[ {}a{} ]]\n",
        "( ".repeat(2_000),
        " )".repeat(2_000)
    ));
    assert_limit_error(&output, "nesting too deep");
}

#[test]
fn deeply_nested_parameter_defaults_do_not_overflow_the_lexer() {
    let output = parse_bash(&format!("echo {}", nested("${a:-", "x", "}", 20_000)));
    assert_eq!(output.file.body.len(), 1);
}

#[test]
fn nested_subscripts_and_arithmetic_parse_in_linear_time() {
    // Each of these parsed its operand both as a word and as arithmetic at
    // every level, doubling the work per level.
    for input in [
        format!("echo {}", nested("${a[", "1", "]}", 60)),
        format!("echo {}", nested("${#a[", "1", "]}", 60)),
        format!("echo {}", nested("${a:", "1", "}", 60)),
        format!("echo {}", nested("$((1+", "1", "))", 60)),
        format!("echo {}", nested("$(( $(echo ", "1", ") ))", 60)),
        format!("a[{}]=1\n", nested("${a[", "1", "]}", 60).trim_end()),
    ] {
        let output = parse_bash(&input);
        assert_eq!(output.file.body.len(), 1, "{input}");
    }
}

#[test]
fn moderately_nested_subscripts_stay_clean() {
    let output = parse_bash(&format!("echo {}", nested("${a[", "1", "]}", 12)));
    assert!(output.is_ok(), "{:?}", output.diagnostics);
}
