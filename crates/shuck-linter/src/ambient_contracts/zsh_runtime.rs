//! Ambient bindings and external consumers supplied by the zsh runtime.
//!
//! zsh plugins, themes, functions, and tests frequently run inside an interactive
//! zsh process that predefines special parameters. They may also assign values
//! that zsh or a zsh test harness consumes later:
//!
//! ```zsh
//! print -r -- "$history" "$widgets"
//! precmd_functions+=(_example_precmd)
//! expected_region_highlight=('1 4 fg=red')
//! ```
//!
//! The provider only applies to zsh-shaped paths or zsh projects so ordinary
//! scripts do not inherit interactive-runtime assumptions.

use shuck_ast::Name;
use shuck_semantic::{ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind};

use super::AmbientContractCollector;
use super::signals::{PathSignals, SourceSignals};
use super::zsh_paths::{
    zsh_dotfile_path_shape, zsh_project_path_shape, zsh_runtime_path_shape,
    zsh_syntax_highlighting_test_path_shape, zsh_test_data_path_shape,
};
use crate::ShellDialect;

pub(super) fn matches_zsh_ambient_runtime_contract(
    collector: &AmbientContractCollector<'_>,
    shell: ShellDialect,
) -> bool {
    let path = collector.path_signals();
    shell_matches_zsh_runtime_context(shell, path)
        && zsh_ambient_runtime_has_signal(collector.source_signals(), path)
}

pub(super) fn apply_zsh_ambient_runtime_contract(
    contract: &mut FileContract,
    collector: &AmbientContractCollector<'_>,
) {
    let path = collector.path_signals();
    let source = collector.source_signals();
    for name in zsh_initialized_runtime_names(source, path) {
        contract.add_provided_binding(ProvidedBinding::new_file_entry_initialized(
            Name::from(name),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }

    for name in zsh_externally_consumed_names(source, path) {
        contract.add_externally_consumed_binding_name(Name::from(name));
    }

    for prefix in zsh_test_fixture_consumed_prefixes(source, path) {
        contract.add_externally_consumed_binding_prefix(Name::from(prefix));
    }
}

fn shell_matches_zsh_runtime_context(shell: ShellDialect, path: &PathSignals) -> bool {
    shell == ShellDialect::Zsh
        || (shell == ShellDialect::Unknown
            && (zsh_dotfile_path_shape(path.lower_path())
                || zsh_project_path_shape(path.lower_path())))
}

fn zsh_ambient_runtime_has_signal(source: &SourceSignals<'_>, path: &PathSignals) -> bool {
    source.mentions_any(ZSH_INITIALIZED_SPECIAL_PARAMETERS)
        || source.mentions_any(ZSH_HOOK_ARRAY_PARAMETERS)
        || zsh_prompt_color_runtime_shape(source, path)
        || !zsh_externally_consumed_names(source, path).is_empty()
        || zsh_test_fixture_consumed_prefixes(source, path)
            .next()
            .is_some()
}

fn zsh_initialized_runtime_names<'a>(
    source: &'a SourceSignals<'_>,
    path: &'a PathSignals,
) -> impl Iterator<Item = &'static str> + 'a {
    ZSH_INITIALIZED_SPECIAL_PARAMETERS
        .iter()
        .copied()
        .filter(move |name| {
            source.mentions_name(name) && zsh_special_parameter_available(name, source, path)
        })
        .chain(
            ZSH_PROMPT_COLOR_PARAMETERS
                .iter()
                .copied()
                .filter(move |name| {
                    source.mentions_name(name) && zsh_prompt_color_runtime_shape(source, path)
                }),
        )
        .chain(
            ZSH_HOOK_ARRAY_PARAMETERS
                .iter()
                .copied()
                .filter(move |name| {
                    source.mentions_name(name) && zsh_runtime_path_shape(path.lower_path())
                }),
        )
}

fn zsh_special_parameter_available(
    name: &str,
    source: &SourceSignals<'_>,
    path: &PathSignals,
) -> bool {
    match name {
        "sysparams" => {
            zsh_runtime_path_shape(path.lower_path()) || source.loads_zsh_module("zsh/system")
        }
        "langinfo" => {
            zsh_runtime_path_shape(path.lower_path()) || source.loads_zsh_module("zsh/langinfo")
        }
        "compstate" | "words" => zsh_runtime_path_shape(path.lower_path()),
        "galiases" | "history" | "keymaps" | "reswords" | "saliases" | "userdirs" => {
            zsh_runtime_path_shape(path.lower_path()) || source.loads_zsh_module("zsh/parameter")
        }
        _ => true,
    }
}

fn zsh_externally_consumed_names(
    source: &SourceSignals<'_>,
    path: &PathSignals,
) -> Vec<&'static str> {
    let runtime_path = zsh_runtime_path_shape(path.lower_path());
    if !runtime_path && !zsh_test_data_path_shape(path.lower_path()) {
        return Vec::new();
    }

    let mut consumed = ZSH_EXTERNALLY_CONSUMED_OUTPUT_PARAMETERS
        .iter()
        .copied()
        .filter(|name| source.assigns_name(name))
        .collect::<Vec<_>>();
    if runtime_path {
        consumed.extend(
            ZSH_HOOK_ARRAY_PARAMETERS
                .iter()
                .copied()
                .filter(|name| source.assigns_name(name)),
        );
    }
    consumed
}

const ZSH_HOOK_ARRAY_PARAMETERS: &[&str] = &[
    "chpwd_functions",
    "periodic_functions",
    "precmd_functions",
    "preexec_functions",
    "zsh_directory_name_functions",
    "zshaddhistory_functions",
    "zshexit_functions",
];

fn zsh_test_fixture_consumed_prefixes<'a>(
    source: &'a SourceSignals<'_>,
    path: &'a PathSignals,
) -> impl Iterator<Item = &'static str> + 'a {
    ["expected_"].into_iter().filter(|prefix| {
        zsh_syntax_highlighting_test_path_shape(path.lower_path()) && source.contains(prefix)
    })
}

const ZSH_INITIALIZED_SPECIAL_PARAMETERS: &[&str] = &[
    "compstate",
    "galiases",
    "history",
    "keymaps",
    "langinfo",
    "parameters",
    "reswords",
    "saliases",
    "sysparams",
    "userdirs",
    "widgets",
    "words",
];

const ZSH_PROMPT_COLOR_PARAMETERS: &[&str] = &[
    "bg",
    "bg_bold",
    "bg_no_bold",
    "color",
    "colour",
    "fg",
    "fg_bold",
    "fg_no_bold",
    "reset_color",
];

const ZSH_EXTERNALLY_CONSUMED_OUTPUT_PARAMETERS: &[&str] =
    &["REPLY", "WORDCHARS", "compstate", "comppostfuncs", "reply"];

fn zsh_prompt_color_runtime_shape(source: &SourceSignals<'_>, path: &PathSignals) -> bool {
    (zsh_runtime_path_shape(path.lower_path()) || source.loads_zsh_colors())
        && source.mentions_any(ZSH_PROMPT_COLOR_PARAMETERS)
}
