mod directive;
mod index;
mod shellcheck_map;

pub(crate) use directive::shellcheck_directive_can_apply_to_following_command;
pub use directive::{SuppressionAction, SuppressionDirective, SuppressionSource, parse_directives};
pub use index::{SuppressionIndex, first_statement_line};
pub use shellcheck_map::ShellCheckCodeMap;
