//! Ambient consumption for zsh configuration namespaces.
//!
//! zsh dotfiles and framework config files often assign names that are read by a
//! loader after the file has been sourced. In those contexts the assignment is
//! intentionally observable even when the variable is not read locally:
//!
//! ```zsh
//! POWERLEVEL9K_LEFT_PROMPT_ELEMENTS=(dir vcs)
//! ZSH_AUTOSUGGEST_STRATEGY=(history)
//! HISTSIZE=10000
//! ```

use std::path::Path;

use shuck_ast::Name;
use shuck_semantic::FileContract;

use super::AmbientContractCollector;
use super::path::{lower_path, path_file_name};
use super::source_scan::source_assigns_name;
use super::zsh_paths::{p10k_config_path_shape, zsh_dotfile_path_shape};
use crate::ShellDialect;

pub(super) fn matches_zsh_config_contract(
    collector: &AmbientContractCollector<'_>,
    path: &Path,
    shell: ShellDialect,
) -> bool {
    let lower = lower_path(path);
    shell_matches_zsh_config_context(shell, &lower)
        && (zsh_config_consumed_prefixes(collector.source, &lower)
            .next()
            .is_some()
            || !zsh_config_consumed_names(collector.source, &lower).is_empty())
}

pub(super) fn build_zsh_config_contract(
    collector: &AmbientContractCollector<'_>,
    path: &Path,
    _shell: ShellDialect,
) -> FileContract {
    let lower = lower_path(path);
    let mut contract = FileContract::default();
    for prefix in zsh_config_consumed_prefixes(collector.source, &lower) {
        contract.add_externally_consumed_binding_prefix(Name::from(prefix));
    }
    for name in zsh_config_consumed_names(collector.source, &lower) {
        contract.add_externally_consumed_binding_name(Name::from(name));
    }
    contract
}

fn shell_matches_zsh_config_context(shell: ShellDialect, lower_path: &str) -> bool {
    shell == ShellDialect::Zsh
        || (shell == ShellDialect::Unknown
            && (p10k_config_path_shape(lower_path) || zsh_dotfile_path_shape(lower_path)))
}

fn zsh_config_consumed_prefixes<'a>(
    source: &'a str,
    lower_path: &'a str,
) -> impl Iterator<Item = &'static str> + 'a {
    [
        "HISTORY_SUBSTRING_SEARCH_",
        "ITERM2_",
        "P9K_",
        "POWERLEVEL9K_",
        "ZDOT_",
        "ZSH_AUTOSUGGEST_",
        "ZSH_HIGHLIGHT_",
    ]
    .into_iter()
    .filter(|prefix| source.contains(prefix) && zsh_config_prefix_path_shape(prefix, lower_path))
}

fn zsh_config_prefix_path_shape(prefix: &str, lower_path: &str) -> bool {
    match prefix {
        "HISTORY_SUBSTRING_SEARCH_" => {
            lower_path.contains("/history-substring-search/")
                || lower_path.contains("/modules/history-substring-search/")
        }
        "ITERM2_" => p10k_config_path_shape(lower_path) || zsh_dotfile_path_shape(lower_path),
        "P9K_" | "POWERLEVEL9K_" => {
            p10k_config_path_shape(lower_path) || zsh_dotfile_path_shape(lower_path)
        }
        "ZDOT_" => zsh_dotfile_path_shape(lower_path),
        "ZSH_AUTOSUGGEST_" => lower_path.contains("/zsh-autosuggestions/"),
        "ZSH_HIGHLIGHT_" => lower_path.contains("/zsh-syntax-highlighting/"),
        _ => false,
    }
}

const ZSH_CONFIG_EXTERNALLY_CONSUMED_NAMES: &[&str] = &["HISTFILE", "HISTSIZE", "SAVEHIST"];

fn zsh_config_consumed_names(source: &str, lower_path: &str) -> Vec<&'static str> {
    ZSH_CONFIG_EXTERNALLY_CONSUMED_NAMES
        .iter()
        .copied()
        .filter(|name| {
            source_assigns_name(source, name)
                && zsh_config_consumed_name_path_shape(name, lower_path)
        })
        .collect()
}

fn zsh_config_consumed_name_path_shape(name: &str, lower_path: &str) -> bool {
    match name {
        "HISTFILE" | "HISTSIZE" | "SAVEHIST" => zsh_history_config_path_shape(lower_path),
        _ => false,
    }
}

fn zsh_history_config_path_shape(lower_path: &str) -> bool {
    let file_name = path_file_name(lower_path);
    zsh_dotfile_path_shape(lower_path)
        || lower_path.contains("/zsh/")
        || lower_path.contains("/modules/history/")
        || matches!(file_name, "config.zsh" | "history.zsh" | "init.zsh")
}
