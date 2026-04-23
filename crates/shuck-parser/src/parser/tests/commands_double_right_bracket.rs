use super::*;

#[test]
fn arithmetic_expansion_command_can_take_double_right_bracket_argument() {
    let input = r#"if false; then
  :
elif [[ "$x" == true ]] && $(( ${#accumulator[@]}%2 )) -eq 0 ]]; then
  :
fi
"#;

    let parsed = Parser::new(input).parse().unwrap();

    assert_eq!(parsed.status, ParseStatus::Clean);
    assert!(parsed.diagnostics.is_empty());
}
