//! External consumption of zsh module metadata declarations.
//!
//! Some zsh module systems discover modules by reading a small metadata triplet
//! and then invoking the configured entry function. The variables look unused
//! inside the file, but the loader reads them after sourcing:
//!
//! ```zsh
//! module_name="package-manager"
//! module_description="Install package manager"
//! module_main_function="run_package_manager_module"
//!
//! run_package_manager_module() {
//!   :
//! }
//! ```

use std::path::Path;

use shuck_ast::Name;
use shuck_semantic::FileContract;

use super::AmbientContractCollector;
use super::source_scan::{
    code_before_shell_comment, is_shell_variable_name, parse_shell_name_at, source_assigns_name,
};
use crate::ShellDialect;

pub(super) fn matches_zsh_module_metadata_contract(
    collector: &AmbientContractCollector<'_>,
    _path: &Path,
    shell: ShellDialect,
) -> bool {
    shell == ShellDialect::Zsh && zsh_module_metadata_source_shape(collector.source)
}

pub(super) fn build_zsh_module_metadata_contract(
    _collector: &AmbientContractCollector<'_>,
    _path: &Path,
    _shell: ShellDialect,
) -> FileContract {
    let mut contract = FileContract::default();
    for name in ZSH_MODULE_METADATA_NAMES {
        contract.add_externally_consumed_binding_name(Name::from(*name));
    }
    contract
}

const ZSH_MODULE_METADATA_NAMES: &[&str] =
    &["module_name", "module_description", "module_main_function"];

fn zsh_module_metadata_source_shape(source: &str) -> bool {
    ZSH_MODULE_METADATA_NAMES
        .iter()
        .all(|name| source_assigns_name(source, name))
        && source_static_assignment_value(source, "module_main_function")
            .is_some_and(|function_name| source_defines_function(source, &function_name))
}

fn source_static_assignment_value(source: &str, name: &str) -> Option<String> {
    for line in source.lines() {
        let code = code_before_shell_comment(line).trim_start();
        let Some(rest) = code.strip_prefix(name) else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(value) = rest.strip_prefix('=') else {
            continue;
        };
        let value = value.trim_start();
        if value.is_empty() {
            continue;
        }

        let (raw_value, quoted) = if let Some(rest) = value.strip_prefix('"') {
            (rest.split('"').next()?, true)
        } else if let Some(rest) = value.strip_prefix('\'') {
            (rest.split('\'').next()?, true)
        } else {
            (
                value
                    .split(|ch: char| ch.is_whitespace() || ch == ';')
                    .next()?,
                false,
            )
        };

        if raw_value.is_empty() || raw_value.contains('$') {
            continue;
        }
        if quoted || is_shell_variable_name(raw_value) {
            return Some(raw_value.to_owned());
        }
    }

    None
}

fn source_defines_function(source: &str, name: &str) -> bool {
    let lines: Vec<_> = source
        .lines()
        .map(|line| code_before_shell_comment(line).trim())
        .collect();
    lines.iter().enumerate().any(|(index, line)| {
        let Some(candidate) = source_function_definition_candidate(line, name) else {
            return false;
        };

        function_definition_rest_opens_body(candidate.rest)
            || (candidate.allows_next_line_body
                && lines
                    .iter()
                    .skip(index + 1)
                    .find(|next| !next.is_empty())
                    .is_some_and(|next| next.starts_with('{')))
    })
}

struct FunctionDefinitionCandidate<'a> {
    rest: &'a str,
    allows_next_line_body: bool,
}

fn source_function_definition_candidate<'a>(
    source: &'a str,
    name: &str,
) -> Option<FunctionDefinitionCandidate<'a>> {
    let (source, has_function_keyword) = source
        .strip_prefix("function ")
        .map_or((source, false), |rest| (rest, true));
    let (candidate, after_name) = parse_shell_name_at(source, 0)?;
    if candidate != name {
        return None;
    }

    let rest = source[after_name..].trim();
    Some(FunctionDefinitionCandidate {
        rest,
        allows_next_line_body: (has_function_keyword && rest.is_empty()) || rest == "()",
    })
}

fn function_definition_rest_opens_body(rest: &str) -> bool {
    rest.starts_with('{') || (rest.starts_with("()") && rest.contains('{'))
}
