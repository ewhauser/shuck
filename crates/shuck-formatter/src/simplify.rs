use shuck_ast::Script;

pub fn simplify_script(_script: &mut Script) {
    // Intentionally conservative for v1: the simplify pass is wired into the
    // formatter pipeline, but it currently skips rewrites unless they are
    // proven byte-stable and idempotent against the parser and formatter.
}
