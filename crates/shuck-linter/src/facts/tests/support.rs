use std::path::Path;

use shuck_indexer::Indexer;
use shuck_parser::parser::{Parser, ShellDialect as ParseShellDialect};
use shuck_semantic::SemanticModel;

use crate::{LinterFacts, ShellDialect, classify_file_context};

pub(super) fn with_facts_dialect(
    source: &str,
    path: Option<&Path>,
    parse_dialect: ParseShellDialect,
    shell: ShellDialect,
    visit: impl FnOnce(&shuck_parser::parser::ParseResult, &LinterFacts<'_>),
) {
    let output = Parser::with_dialect(source, parse_dialect).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let file_context = classify_file_context(source, path, shell);
    let facts = LinterFacts::build(&output.file, source, &semantic, &indexer, &file_context);
    visit(&output, &facts);
}

pub(super) fn with_facts(
    source: &str,
    path: Option<&Path>,
    visit: impl FnOnce(&shuck_parser::parser::ParseResult, &LinterFacts<'_>),
) {
    with_facts_dialect(
        source,
        path,
        ParseShellDialect::Bash,
        ShellDialect::Bash,
        visit,
    );
}
