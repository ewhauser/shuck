mod directive;
mod index;
mod rewrite;
mod shellcheck_map;

pub(crate) use directive::parse_directives;
pub(crate) use directive::shellcheck_directive_can_apply_to_following_command;
pub use directive::{SuppressionAction, SuppressionDirective, SuppressionSource};
pub use index::SuppressionIndex;
pub(crate) use index::{
    first_statement_line, sort_command_spans_for_lookup, statement_suppression_span,
};
pub use rewrite::{AddIgnoreParseError, AddIgnoreResult, add_ignores_to_path};
pub use shellcheck_map::ShellCheckCodeMap;
