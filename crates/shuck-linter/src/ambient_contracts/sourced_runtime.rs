//! Ambient bindings for runtime helper files that are sourced by a framework.
//!
//! This provider covers shells that load a helper into a caller-owned runtime
//! where some variables are initialized by that runtime rather than by the file
//! itself. The most specific current examples are bash-it themes and bash
//! completion helpers.
//!
//! Bash-it themes can read palette variables injected by the theme runtime:
//!
//! ```sh
//! prompt_command() {
//!   PS1="${green}${reset_color}"
//! }
//! PROMPT_COMMAND=prompt_command
//! ```
//!
//! Bash completion helpers can read completion state after calling an initializer
//! that mutates the current shell:
//!
//! ```sh
//! _example() {
//!   _init_completion || return
//!   printf '%s\n' "$cur" "$cword" "$comp_args"
//! }
//! ```

use std::collections::BTreeSet;

use shuck_ast::{
    Name, NormalizedCommand, Word, WrapperKind, normalize_command_words, static_word_text,
};
use shuck_semantic::{ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind};

use super::AmbientContractCollector;
use super::signals::PathSignals;
use super::source_scan::shell_assignment_token;
use crate::ShellDialect;

pub(super) fn matches_sourced_runtime_contract(
    collector: &AmbientContractCollector<'_>,
    _shell: ShellDialect,
) -> bool {
    sourced_runtime_path_shape(collector.path_signals()) && sourced_runtime_source_shape(collector)
}

pub(super) fn apply_sourced_runtime_contract(
    contract: &mut FileContract,
    collector: &AmbientContractCollector<'_>,
) {
    let mut names = BTreeSet::new();

    for name in runtime_names_for_source_path(collector) {
        names.insert((*name).to_owned());
    }

    for name in names {
        contract.add_provided_binding(ProvidedBinding::new(
            Name::from(name.as_str()),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }
}

fn sourced_runtime_path_shape(path: &PathSignals) -> bool {
    path.matches_any(&[
        "/completion/",
        "/completions/",
        ".completion.",
        "bash_autocomplete",
        "/themes/",
        ".theme.",
        "/plugins/",
        "/plugin/",
        "/modules/",
        "/scriptmodules/",
        "/scripts/functions/",
        "/rvm/scripts/",
        "/lgsm/modules/",
        "/common/environment/setup/",
        "/common/chroot-style/",
        "/common/hooks/",
        "termux-packages/packages/",
    ])
}

fn sourced_runtime_source_shape(collector: &AmbientContractCollector<'_>) -> bool {
    let source = collector.source_signals();
    source.has_probable_function_definition()
        || source.has_source_command()
        || source.contains("PROMPT_COMMAND")
        || source.contains("COMPREPLY")
        || source.contains("about-completion")
        || (collector.path_signals().contains("termux-packages") && source.contains("TERMUX_"))
        || collector.completion_initializer_invoked
}

fn runtime_names_for_source_path(
    collector: &AmbientContractCollector<'_>,
) -> &'static [&'static str] {
    if bash_it_theme_runtime_shape(collector) {
        return &[
            "black",
            "red",
            "green",
            "yellow",
            "blue",
            "purple",
            "cyan",
            "white",
            "normal",
            "default",
            "reset_color",
            "bold_black",
            "bold_red",
            "bold_green",
            "bold_yellow",
            "bold_blue",
            "bold_purple",
            "bold_cyan",
            "bold_white",
            "italic",
        ];
    }

    if completion_runtime_shape(collector) {
        return &["cur", "prev", "words", "cword", "comp_args", "split"];
    }

    &[]
}

fn bash_it_theme_runtime_shape(collector: &AmbientContractCollector<'_>) -> bool {
    let source = collector.source_signals();
    collector
        .path_signals()
        .matches_any(&["/bash-it/themes/", "/bash-it/theme/"])
        && (source.contains("PROMPT_COMMAND")
            || source.contains("SCM_THEME_PROMPT")
            || source.mentions_any(&[
                "black",
                "red",
                "green",
                "yellow",
                "blue",
                "purple",
                "cyan",
                "white",
                "normal",
                "default",
                "reset_color",
                "bold_black",
                "bold_red",
                "bold_green",
                "bold_yellow",
                "bold_blue",
                "bold_purple",
                "bold_cyan",
                "bold_white",
                "italic",
            ]))
}

fn completion_runtime_shape(collector: &AmbientContractCollector<'_>) -> bool {
    completion_runtime_path_shape(collector.path_signals())
        && collector.completion_initializer_invoked
}

fn completion_runtime_path_shape(path: &PathSignals) -> bool {
    path.matches_any(&[
        "/bash-completion/",
        "/bash_completion/",
        "/bash-it/completion/",
        "/bash-it/completions/",
        "/bash-progcomp/",
        "bash_autocomplete",
    ])
}

pub(super) fn normalized_command_invokes_completion_initializer(
    command: &NormalizedCommand<'_>,
    source: &str,
) -> bool {
    if command
        .effective_name
        .as_deref()
        .is_some_and(is_completion_initializer_command)
        && command
            .wrappers
            .iter()
            .all(wrapper_can_affect_current_shell)
    {
        return true;
    }

    if command.effective_name.as_deref() != Some("env")
        || !command
            .wrappers
            .iter()
            .all(wrapper_can_affect_current_shell)
    {
        return false;
    }

    command
        .body_args()
        .iter()
        .enumerate()
        .find_map(|(index, word)| {
            let text = static_word_text(word, source)?;
            (!shell_assignment_token(text.as_ref())).then_some(index)
        })
        .and_then(|index| command.body_args().get(index..))
        .and_then(|words| normalized_words_invoke_completion_initializer(words, source))
        .unwrap_or(false)
}

fn normalized_words_invoke_completion_initializer(words: &[&Word], source: &str) -> Option<bool> {
    let command = normalize_command_words(words, source)?;
    Some(normalized_command_invokes_completion_initializer(
        &command, source,
    ))
}

fn wrapper_can_affect_current_shell(wrapper: &WrapperKind) -> bool {
    matches!(
        wrapper,
        WrapperKind::Command | WrapperKind::Builtin | WrapperKind::Exec | WrapperKind::Noglob
    )
}

fn is_completion_initializer_command(token: &str) -> bool {
    matches!(
        token,
        "_init_completion" | "_get_comp_words_by_ref" | "_comp_initialize" | "about-completion"
    )
}
