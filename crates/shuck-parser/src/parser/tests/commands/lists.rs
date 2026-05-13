use super::*;

#[test]
fn test_parse_pipeline() {
    let parser = Parser::new("echo hello | cat");
    let script = parser.parse().unwrap().file;

    assert_eq!(script.body.len(), 1);
    let pipeline = expect_binary(&script.body[0]);
    assert_eq!(pipeline.op, BinaryOp::Pipe);
    assert_eq!(
        expect_simple(&pipeline.left)
            .name
            .render("echo hello | cat"),
        "echo"
    );
    assert_eq!(
        expect_simple(&pipeline.right)
            .name
            .render("echo hello | cat"),
        "cat"
    );
}

#[test]
fn test_parse_pipe_both_pipeline() {
    let input = "echo hello |& cat";
    let script = Parser::new(input).parse().unwrap().file;

    let pipeline = expect_binary(&script.body[0]);
    assert_eq!(pipeline.op, BinaryOp::PipeAll);
    assert_eq!(expect_simple(&pipeline.left).name.render(input), "echo");
    assert_eq!(expect_simple(&pipeline.right).name.render(input), "cat");
}

#[test]
fn test_parse_command_list_and() {
    let parser = Parser::new("true && echo success");
    let script = parser.parse().unwrap().file;

    assert_eq!(expect_binary(&script.body[0]).op, BinaryOp::And);
}

#[test]
fn test_parse_command_list_or() {
    let parser = Parser::new("false || echo fallback");
    let script = parser.parse().unwrap().file;

    assert_eq!(expect_binary(&script.body[0]).op, BinaryOp::Or);
}

#[test]
fn test_parse_command_list_preserves_operator_spans() {
    let input = "true && false || echo fallback";
    let script = Parser::new(input).parse().unwrap().file;

    let outer = expect_binary(&script.body[0]);
    assert_eq!(outer.op, BinaryOp::Or);
    assert_eq!(outer.op_span.slice(input), "||");
    let inner = expect_binary(&outer.left);
    assert_eq!(inner.op, BinaryOp::And);
    assert_eq!(inner.op_span.slice(input), "&&");
}
