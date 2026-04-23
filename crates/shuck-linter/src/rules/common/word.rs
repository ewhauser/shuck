use shuck_ast::ConditionalBinaryOp;

pub fn conditional_binary_op_is_string_match(op: ConditionalBinaryOp) -> bool {
    matches!(
        op,
        ConditionalBinaryOp::PatternEqShort
            | ConditionalBinaryOp::PatternEq
            | ConditionalBinaryOp::PatternNe
    )
}
