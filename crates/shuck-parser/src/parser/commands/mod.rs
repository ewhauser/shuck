use super::*;
use smallvec::SmallVec;

mod case;
mod compound;
mod conditionals;
mod functions;
mod if_clause;
mod lists;
mod loops;
mod simple;

#[derive(Debug, Clone, Copy)]
enum ForHeaderSurface {
    In {
        in_span: Option<Span>,
    },
    Paren {
        left_paren_span: Span,
        right_paren_span: Span,
    },
}

#[derive(Debug, Clone, Copy)]
struct ZshCaseScanState {
    position: Position,
    paren_depth: usize,
    bracket_depth: usize,
    brace_depth: usize,
    in_single: bool,
    in_double: bool,
    in_backtick: bool,
    escaped: bool,
}

impl ZshCaseScanState {
    fn new(position: Position) -> Self {
        Self {
            position,
            paren_depth: 0,
            bracket_depth: 0,
            brace_depth: 0,
            in_single: false,
            in_double: false,
            in_backtick: false,
            escaped: false,
        }
    }
}
