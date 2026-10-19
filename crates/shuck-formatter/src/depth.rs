//! Guard against ASTs nested deeper than the formatter can recurse through.
//!
//! Most formatter passes walk statements recursively, and a long `&&`, `||`,
//! or `|` chain is a left-nested tree whose depth equals its length. Rather
//! than overflowing the stack on such input, formatting is refused up front.

use shuck_ast::{File, Stmt};

use crate::visit::{AstVisitor, walk_stmt};

/// Maximum number of statements nested inside one another, counting each
/// operand of a binary list or pipeline as one level.
pub(crate) const MAX_STATEMENT_NESTING: usize = 1000;

/// Returns `true` when `file` nests statements deeper than
/// [`MAX_STATEMENT_NESTING`]. The walk stops descending at the limit, so the
/// check itself never recurses deeper than the formatter would.
pub(crate) fn exceeds_statement_nesting(file: &File) -> bool {
    let mut probe = DepthProbe {
        depth: 0,
        exceeded: false,
    };
    probe.visit_stmt_seq(&file.body);
    probe.exceeded
}

struct DepthProbe {
    depth: usize,
    exceeded: bool,
}

impl AstVisitor for DepthProbe {
    fn visit_stmt(&mut self, stmt: &Stmt) {
        if self.exceeded {
            return;
        }
        if self.depth >= MAX_STATEMENT_NESTING {
            self.exceeded = true;
            return;
        }
        self.depth += 1;
        walk_stmt(self, stmt);
        self.depth -= 1;
    }
}

#[cfg(test)]
mod tests {
    use shuck_parser::parser::Parser;

    use super::*;

    fn exceeds(source: &str) -> bool {
        let source = source.to_string();
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || exceeds_statement_nesting(&Parser::new(&source).parse().unwrap().file))
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn accepts_moderate_lists() {
        assert!(!exceeds(&format!("{}\n", ["a"; 500].join(" && "))));
    }

    #[test]
    fn rejects_lists_beyond_the_limit() {
        assert!(exceeds(&format!("{}\n", ["a"; 1500].join(" && "))));
        assert!(exceeds(&format!("{}\n", ["a"; 1500].join(" | "))));
    }
}
