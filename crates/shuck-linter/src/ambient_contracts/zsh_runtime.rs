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

use std::path::Path;

use shuck_ast::Name;
use shuck_semantic::{ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind};

use super::AmbientContractCollector;
use super::path::lower_path;
use super::source_scan::{source_assigns_name, source_mentions_any, source_mentions_name};
use super::zsh_paths::{
    zsh_dotfile_path_shape, zsh_project_path_shape, zsh_runtime_path_shape,
    zsh_syntax_highlighting_test_path_shape, zsh_test_data_path_shape,
};
use crate::ShellDialect;

pub(super) fn matches_zsh_ambient_runtime_contract(
    collector: &AmbientContractCollector<'_>,
    path: &Path,
    shell: ShellDialect,
) -> bool {
    let lower = lower_path(path);
    shell_matches_zsh_runtime_context(shell, &lower)
        && zsh_ambient_runtime_has_signal(collector.source, &lower)
}

pub(super) fn build_zsh_ambient_runtime_contract(
    collector: &AmbientContractCollector<'_>,
    path: &Path,
    _shell: ShellDialect,
) -> FileContract {
    let lower = lower_path(path);
    let source = collector.source;
    let mut contract = FileContract::default();

    for name in zsh_initialized_runtime_names(source, &lower) {
        contract.add_provided_binding(ProvidedBinding::new_file_entry_initialized(
            Name::from(name),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }

    for name in zsh_externally_consumed_names(source, &lower) {
        contract.add_externally_consumed_binding_name(Name::from(name));
    }

    for prefix in zsh_test_fixture_consumed_prefixes(source, &lower) {
        contract.add_externally_consumed_binding_prefix(Name::from(prefix));
    }

    contract
}

fn shell_matches_zsh_runtime_context(shell: ShellDialect, lower_path: &str) -> bool {
    shell == ShellDialect::Zsh
        || (shell == ShellDialect::Unknown
            && (zsh_dotfile_path_shape(lower_path) || zsh_project_path_shape(lower_path)))
}

fn zsh_ambient_runtime_has_signal(source: &str, lower_path: &str) -> bool {
    source_mentions_any(source, ZSH_INITIALIZED_SPECIAL_PARAMETERS)
        || source_mentions_any(source, ZSH_HOOK_ARRAY_PARAMETERS)
        || zsh_prompt_color_runtime_shape(source, lower_path)
        || !zsh_externally_consumed_names(source, lower_path).is_empty()
        || zsh_test_fixture_consumed_prefixes(source, lower_path)
            .next()
            .is_some()
}

fn zsh_initialized_runtime_names<'a>(
    source: &'a str,
    lower_path: &'a str,
) -> impl Iterator<Item = &'static str> + 'a {
    ZSH_INITIALIZED_SPECIAL_PARAMETERS
        .iter()
        .copied()
        .filter(move |name| {
            source_mentions_name(source, name)
                && zsh_special_parameter_available(name, source, lower_path)
        })
        .chain(
            ZSH_PROMPT_COLOR_PARAMETERS
                .iter()
                .copied()
                .filter(move |name| {
                    source_mentions_name(source, name)
                        && zsh_prompt_color_runtime_shape(source, lower_path)
                }),
        )
        .chain(
            ZSH_HOOK_ARRAY_PARAMETERS
                .iter()
                .copied()
                .filter(move |name| {
                    source_mentions_name(source, name) && zsh_runtime_path_shape(lower_path)
                }),
        )
}

fn zsh_special_parameter_available(name: &str, source: &str, lower_path: &str) -> bool {
    match name {
        "sysparams" => {
            zsh_runtime_path_shape(lower_path) || source_loads_zsh_module(source, "zsh/system")
        }
        "langinfo" => {
            zsh_runtime_path_shape(lower_path) || source_loads_zsh_module(source, "zsh/langinfo")
        }
        "compstate" | "words" => zsh_runtime_path_shape(lower_path),
        "galiases" | "history" | "keymaps" | "reswords" | "saliases" => {
            zsh_runtime_path_shape(lower_path) || source_loads_zsh_module(source, "zsh/parameter")
        }
        _ => true,
    }
}

fn source_loads_zsh_module(source: &str, module: &str) -> bool {
    source.lines().any(|line| {
        line.split('#')
            .next()
            .is_some_and(|code| code.contains("zmodload") && code.contains(module))
    })
}

fn zsh_externally_consumed_names(source: &str, lower_path: &str) -> Vec<&'static str> {
    let runtime_path = zsh_runtime_path_shape(lower_path);
    if !runtime_path && !zsh_test_data_path_shape(lower_path) {
        return Vec::new();
    }

    let mut consumed = ZSH_EXTERNALLY_CONSUMED_OUTPUT_PARAMETERS
        .iter()
        .copied()
        .filter(|name| source_assigns_name(source, name))
        .collect::<Vec<_>>();
    if runtime_path {
        consumed.extend(
            ZSH_HOOK_ARRAY_PARAMETERS
                .iter()
                .copied()
                .filter(|name| source_assigns_name(source, name)),
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
    source: &'a str,
    lower_path: &'a str,
) -> impl Iterator<Item = &'static str> + 'a {
    ["expected_"].into_iter().filter(|prefix| {
        zsh_syntax_highlighting_test_path_shape(lower_path) && source.contains(prefix)
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
    &["REPLY", "compstate", "comppostfuncs", "reply"];

fn zsh_prompt_color_runtime_shape(source: &str, lower_path: &str) -> bool {
    (zsh_runtime_path_shape(lower_path) || source_loads_zsh_colors(source))
        && source_mentions_any(source, ZSH_PROMPT_COLOR_PARAMETERS)
}

fn source_loads_zsh_colors(source: &str) -> bool {
    source.lines().any(|line| {
        line.split('#')
            .next()
            .map(str::trim_start)
            .is_some_and(line_autoloads_zsh_colors)
    })
}

fn line_autoloads_zsh_colors(code: &str) -> bool {
    let code = first_shell_command_segment(code);
    let mut words = code.split_whitespace();
    let mut command = words.next();
    while matches!(command, Some("builtin" | "command")) {
        command = words.next();
    }
    if command != Some("autoload") {
        return false;
    }

    words.any(|word| word == "colors")
}

fn first_shell_command_segment(code: &str) -> &str {
    ["&&", "||", ";", "|"]
        .iter()
        .filter_map(|separator| code.find(separator))
        .min()
        .map_or(code, |index| &code[..index])
}
