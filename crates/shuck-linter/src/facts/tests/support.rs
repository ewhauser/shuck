use std::path::Path;

use shuck_indexer::Indexer;
use shuck_parser::parser::{Parser, ShellDialect as ParseShellDialect};
use shuck_semantic::SemanticModel;

use crate::{AmbientShellOptions, LinterFacts, ShellDialect};

pub(super) fn with_facts_dialect(
    source: &str,
    _path: Option<&Path>,
    parse_dialect: ParseShellDialect,
    shell: ShellDialect,
    visit: impl FnOnce(&shuck_parser::parser::ParseResult, &LinterFacts<'_>),
) {
    let output = Parser::with_dialect(source, parse_dialect).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let semantic = SemanticModel::build(&output.file, source, &indexer);
    let facts = LinterFacts::build_with_shell_and_ambient_shell_options(
        &output.file,
        source,
        &semantic,
        &indexer,
        shell,
        AmbientShellOptions::default(),
    );
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
