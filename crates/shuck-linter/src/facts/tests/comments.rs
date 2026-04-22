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

#[test]
fn captures_c075_move_span_with_existing_newline() {
    let source = "\n#!/bin/sh\n";

    with_facts(source, None, |_, facts| {
        let anchor = facts
            .shebang_not_on_first_line_span()
            .expect("expected C075 anchor span");
        let fix_span = facts
            .shebang_not_on_first_line_fix_span()
            .expect("expected C075 move span");

        assert_eq!(anchor.start.line, 2);
        assert_eq!(anchor.start.column, 1);
        assert_eq!(fix_span.slice(source), "#!/bin/sh\n");
        assert_eq!(
            facts.shebang_not_on_first_line_preferred_newline(),
            Some("\n")
        );
    });
}

#[test]
fn captures_c075_preferred_newline_for_eof_shebangs() {
    let source = "# comment\r\n#!/bin/sh";

    with_facts(source, None, |_, facts| {
        let fix_span = facts
            .shebang_not_on_first_line_fix_span()
            .expect("expected C075 move span");

        assert_eq!(fix_span.slice(source), "#!/bin/sh");
        assert_eq!(
            facts.shebang_not_on_first_line_preferred_newline(),
            Some("\r\n")
        );
    });
}
