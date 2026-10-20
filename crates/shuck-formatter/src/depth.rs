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
    use shuck_ast::Command;
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

    /// Builds `a OP a OP ... a` with `operands` operands directly. The parser
    /// refuses chains this long, but the guard must still hold for any AST it
    /// is handed.
    fn exceeds_chain(op: &str, operands: usize) -> bool {
        let op = op.to_string();
        std::thread::Builder::new()
            .stack_size(256 << 20)
            .spawn(move || {
                let mut file = Parser::new(&format!("a {op} a\n")).parse().unwrap().file;
                let seed = file.body.stmts.pop().unwrap();
                let Command::Binary(binary) = &seed.command else {
                    unreachable!("seed is a binary command");
                };
                let operand = (*binary.right).clone();
                let mut chain = seed.clone();
                for _ in 2..operands {
                    let mut link = seed.clone();
                    let Command::Binary(binary) = &mut link.command else {
                        unreachable!("seed is a binary command");
                    };
                    *binary.left = chain;
                    *binary.right = operand.clone();
                    chain = link;
                }
                file.body.stmts.push(chain);
                exceeds_statement_nesting(&file)
            })
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn rejects_lists_beyond_the_limit() {
        assert!(!exceeds_chain("&&", 500));
        assert!(exceeds_chain("&&", 1500));
        assert!(exceeds_chain("|", 1500));
    }
}
