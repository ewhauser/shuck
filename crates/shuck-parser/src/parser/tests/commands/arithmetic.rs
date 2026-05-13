use super::*;

#[test]
fn test_parse_arithmetic_command_preserves_exact_spans() {
    let input = "(( 1 +\n 2 <= 3 ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(command.expr_span.unwrap().slice(input), " 1 +\n 2 <= 3 ");
    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected binary arithmetic expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::LessThanOrEqual);
    let ArithmeticExpr::Binary {
        left: add_left,
        op: add_op,
        right: add_right,
    } = &left.kind
    else {
        panic!("expected additive left operand");
    };
    assert_eq!(*add_op, ArithmeticBinaryOp::Add);
    expect_number(add_left, input, "1");
    expect_number(add_right, input, "2");
    expect_number(right, input, "3");
}

#[test]
fn test_parse_empty_arithmetic_command_keeps_span_without_typed_ast() {
    let input = "((   ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.expr_span.unwrap().slice(input), "   ");
    assert!(command.expr_ast.is_none());
}

#[test]
fn test_parse_dynamic_arithmetic_command_keeps_compound_shape_with_typed_ast() {
    let input = "((proc[selected]==(1${filter:++1})-proc[start]))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(
        command.expr_span.unwrap().slice(input),
        "proc[selected]==(1${filter:++1})-proc[start]"
    );

    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary { left, op, right } = &expr.kind else {
        panic!("expected comparison expression");
    };
    assert_eq!(*op, ArithmeticBinaryOp::Equal);

    let ArithmeticExpr::Indexed { name, index } = &left.kind else {
        panic!("expected indexed left operand");
    };
    assert_eq!(name, "proc");
    expect_variable(index, "selected");

    let ArithmeticExpr::Binary {
        left: subtract_left,
        op: subtract_op,
        right: subtract_right,
    } = &right.kind
    else {
        panic!("expected subtraction on comparison right-hand side");
    };
    assert_eq!(*subtract_op, ArithmeticBinaryOp::Subtract);

    let ArithmeticExpr::Parenthesized { expression } = &subtract_left.kind else {
        panic!("expected grouped dynamic numeric literal");
    };
    expect_shell_word(expression, input, "1${filter:++1}");

    let ArithmeticExpr::Indexed { name, index } = &subtract_right.kind else {
        panic!("expected indexed right operand");
    };
    assert_eq!(name, "proc");
    expect_variable(index, "start");
}

#[test]
fn test_parse_arithmetic_command_with_nested_parens_and_double_right_paren() {
    let input = "(( (previous_pipe_index > 0) && (previous_pipe_index == ($# - 1)) ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(
        command.expr_span.unwrap().slice(input),
        " (previous_pipe_index > 0) && (previous_pipe_index == ($# - 1)) "
    );
}

#[test]
fn test_parse_arithmetic_command_with_nested_parens_before_outer_close() {
    let input = "(( a <= (1 || 2)))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(command.expr_span.unwrap().slice(input), " a <= (1 || 2)");
}

#[test]
fn test_parse_arithmetic_command_with_grouped_term_before_logical_and() {
    let input = "((threads>(cpu_height-3)*3 && tty_width>=200))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(
        command.expr_span.unwrap().slice(input),
        "threads>(cpu_height-3)*3 && tty_width>=200"
    );
}

#[test]
fn test_parse_arithmetic_command_with_nested_double_parens_and_grouping() {
    let input = "(( x = ((1 + 2) * (3 - 4)) ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    assert!(redirects.is_empty());
    assert_eq!(command.left_paren_span.slice(input), "((");
    assert_eq!(command.right_paren_span.slice(input), "))");
    assert_eq!(
        command.expr_span.unwrap().slice(input),
        " x = ((1 + 2) * (3 - 4)) "
    );

    let ArithmeticExpr::Assignment { target, op, value } = &command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST")
        .kind
    else {
        panic!("expected arithmetic assignment");
    };
    assert_eq!(*op, ArithmeticAssignOp::Assign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable assignment target");
    };
    assert_eq!(name, "x");
    assert!(matches!(value.kind, ArithmeticExpr::Parenthesized { .. }));
}

#[test]
fn test_parse_arithmetic_command_respects_precedence_and_associativity() {
    let input = "(( a + b * c ** d ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary {
        left,
        op: add_op,
        right,
    } = &expr.kind
    else {
        panic!("expected additive expression");
    };
    assert_eq!(*add_op, ArithmeticBinaryOp::Add);
    expect_variable(left, "a");

    let ArithmeticExpr::Binary {
        left: mul_left,
        op: mul_op,
        right: mul_right,
    } = &right.kind
    else {
        panic!("expected multiplicative expression");
    };
    assert_eq!(*mul_op, ArithmeticBinaryOp::Multiply);
    expect_variable(mul_left, "b");

    let ArithmeticExpr::Binary {
        left: pow_left,
        op: pow_op,
        right: pow_right,
    } = &mul_right.kind
    else {
        panic!("expected power expression");
    };
    assert_eq!(*pow_op, ArithmeticBinaryOp::Power);
    expect_variable(pow_left, "c");
    expect_variable(pow_right, "d");
}

#[test]
fn test_parse_arithmetic_command_parses_updates_ternary_and_comma() {
    let input = "(( ++i ? j-- : (k = 1), m ))\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, _) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Arithmetic(command) = compound else {
        panic!("expected arithmetic compound command");
    };

    let expr = command
        .expr_ast
        .as_ref()
        .expect("expected typed arithmetic AST");
    let ArithmeticExpr::Binary {
        left,
        op: comma_op,
        right,
    } = &expr.kind
    else {
        panic!("expected comma expression");
    };
    assert_eq!(*comma_op, ArithmeticBinaryOp::Comma);
    expect_variable(right, "m");

    let ArithmeticExpr::Conditional {
        condition,
        then_expr,
        else_expr,
    } = &left.kind
    else {
        panic!("expected conditional expression");
    };

    let ArithmeticExpr::Unary { op: unary_op, expr } = &condition.kind else {
        panic!("expected prefix update condition");
    };
    assert_eq!(*unary_op, ArithmeticUnaryOp::PreIncrement);
    expect_variable(expr, "i");

    let ArithmeticExpr::Postfix {
        expr,
        op: postfix_op,
    } = &then_expr.kind
    else {
        panic!("expected postfix update in then branch");
    };
    assert_eq!(*postfix_op, ArithmeticPostfixOp::Decrement);
    expect_variable(expr, "j");

    let ArithmeticExpr::Parenthesized { expression } = &else_expr.kind else {
        panic!("expected parenthesized else branch");
    };
    let ArithmeticExpr::Assignment { target, op, value } = &expression.kind else {
        panic!("expected assignment inside else branch");
    };
    assert_eq!(*op, ArithmeticAssignOp::Assign);
    let ArithmeticLvalue::Variable(name) = target else {
        panic!("expected variable else target");
    };
    assert_eq!(name, "k");
    expect_number(value, input, "1");
}

#[test]
fn test_double_left_paren_command_closed_with_spaced_right_parens_parses_as_subshells() {
    let input = "(( echo 1\necho 2\n(( x ))\n: $(( x ))\necho 3\n) )\n";
    let script = Parser::new(input).parse().unwrap().file;

    let (compound, redirects) = expect_compound(&script.body[0]);
    let AstCompoundCommand::Subshell(commands) = compound else {
        panic!("expected outer subshell");
    };
    assert!(redirects.is_empty());
    assert_eq!(commands.len(), 1);
    assert!(matches!(
        commands[0].command,
        AstCommand::Compound(AstCompoundCommand::Subshell(_))
    ));
}
