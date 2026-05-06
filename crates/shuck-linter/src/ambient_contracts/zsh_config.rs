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

use shuck_ast::Name;
use shuck_semantic::FileContract;

use super::AmbientContractCollector;
use super::signals::{PathSignals, SourceSignals};
use super::zsh_paths::{p10k_config_path_shape, zsh_dotfile_path_shape};
use crate::ShellDialect;

pub(super) fn matches_zsh_config_contract(
    collector: &AmbientContractCollector<'_>,
    shell: ShellDialect,
) -> bool {
    let path = collector.path_signals();
    let source = collector.source_signals();
    shell_matches_zsh_config_context(shell, path)
        && (zsh_config_consumed_prefixes(source, path).next().is_some()
            || !zsh_config_consumed_names(source, path).is_empty())
}

pub(super) fn build_zsh_config_contract(
    collector: &AmbientContractCollector<'_>,
    _shell: ShellDialect,
) -> FileContract {
    let path = collector.path_signals();
    let source = collector.source_signals();
    let mut contract = FileContract::default();
    for prefix in zsh_config_consumed_prefixes(source, path) {
        contract.add_externally_consumed_binding_prefix(Name::from(prefix));
    }
    for name in zsh_config_consumed_names(source, path) {
        contract.add_externally_consumed_binding_name(Name::from(name));
    }
    contract
}

fn shell_matches_zsh_config_context(shell: ShellDialect, path: &PathSignals) -> bool {
    shell == ShellDialect::Zsh
        || (shell == ShellDialect::Unknown
            && (p10k_config_path_shape(path.lower_path())
                || zsh_dotfile_path_shape(path.lower_path())))
}

fn zsh_config_consumed_prefixes<'a>(
    source: &'a SourceSignals<'_>,
    path: &'a PathSignals,
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
    .filter(|prefix| source.contains(prefix) && zsh_config_prefix_path_shape(prefix, path))
}

fn zsh_config_prefix_path_shape(prefix: &str, path: &PathSignals) -> bool {
    match prefix {
        "HISTORY_SUBSTRING_SEARCH_" => {
            path.contains("/history-substring-search/")
                || path.contains("/modules/history-substring-search/")
        }
        "ITERM2_" => {
            p10k_config_path_shape(path.lower_path()) || zsh_dotfile_path_shape(path.lower_path())
        }
        "P9K_" | "POWERLEVEL9K_" => {
            p10k_config_path_shape(path.lower_path()) || zsh_dotfile_path_shape(path.lower_path())
        }
        "ZDOT_" => zsh_dotfile_path_shape(path.lower_path()),
        "ZSH_AUTOSUGGEST_" => path.contains("/zsh-autosuggestions/"),
        "ZSH_HIGHLIGHT_" => path.contains("/zsh-syntax-highlighting/"),
        _ => false,
    }
}

const ZSH_CONFIG_EXTERNALLY_CONSUMED_NAMES: &[&str] = &["HISTFILE", "HISTSIZE", "SAVEHIST"];

fn zsh_config_consumed_names(source: &SourceSignals<'_>, path: &PathSignals) -> Vec<&'static str> {
    ZSH_CONFIG_EXTERNALLY_CONSUMED_NAMES
        .iter()
        .copied()
        .filter(|name| source.assigns_name(name) && zsh_config_consumed_name_path_shape(name, path))
        .collect()
}

fn zsh_config_consumed_name_path_shape(name: &str, path: &PathSignals) -> bool {
    match name {
        "HISTFILE" | "HISTSIZE" | "SAVEHIST" => zsh_history_config_path_shape(path),
        _ => false,
    }
}

fn zsh_history_config_path_shape(path: &PathSignals) -> bool {
    let file_name = path.file_name();
    zsh_dotfile_path_shape(path.lower_path())
        || path.contains("/zsh/")
        || path.contains("/modules/history/")
        || matches!(file_name, "config.zsh" | "history.zsh" | "init.zsh")
}
