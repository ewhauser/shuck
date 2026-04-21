use super::with_facts;

#[test]
fn captures_c074_anchor_and_whitespace_span() {
    let source = "# \t!/bin/sh\n";

    with_facts(source, None, |_, facts| {
        let anchor = facts
            .space_after_hash_bang_span()
            .expect("expected C074 anchor span");
        let whitespace = facts
            .space_after_hash_bang_whitespace_span()
            .expect("expected C074 whitespace span");

        assert_eq!(anchor.start.line, 1);
        assert_eq!(anchor.start.column, 2);
        assert_eq!(whitespace.start.column, 2);
        assert_eq!(whitespace.end.column, 4);
        assert_eq!(whitespace.slice(source), " \t");
    });
}

#[test]
fn ignores_non_header_like_c074_lines_in_facts() {
    with_facts("echo ok\n# !/bin/sh\n", None, |_, facts| {
        assert!(facts.space_after_hash_bang_span().is_none());
        assert!(facts.space_after_hash_bang_whitespace_span().is_none());
    });
}
