use shuck_ast::Span;
use shuck_indexer::Indexer;
use shuck_parser::{
    ShellDialect as ParseDialect,
    parser::{ParseResult, Parser},
};

pub(crate) const PARSER_DIALECTS: [ParseDialect; 4] = [
    ParseDialect::Bash,
    ParseDialect::Posix,
    ParseDialect::Mksh,
    ParseDialect::Zsh,
];

pub(crate) fn assert_span_valid(span: Span, source: &str) {
    assert!(
        span.start.offset <= span.end.offset,
        "invalid span ordering: {:?}",
        span
    );
    assert!(
        span.end.offset <= source.len(),
        "span end is out of bounds: {:?}",
        span
    );
    assert!(
        source.is_char_boundary(span.start.offset),
        "span start is not a char boundary: {:?}",
        span
    );
    assert!(
        source.is_char_boundary(span.end.offset),
        "span end is not a char boundary: {:?}",
        span
    );
}

pub(crate) fn recovered_parse_and_index(
    source: &str,
    dialect: ParseDialect,
) -> (ParseResult, Indexer) {
    let parse_result = Parser::with_dialect(source, dialect).parse();
    let indexer = Indexer::new(source, &parse_result);
    (parse_result, indexer)
}
