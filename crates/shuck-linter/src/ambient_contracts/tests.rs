use std::path::Path;

use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;
use shuck_semantic::{
    FileContract, FileEntryContractCollector, NoopTraversalObserver, PluginFramework,
    PluginRequest, PluginRequestKind, SemanticBuildOptions, build_with_observer_with_options,
};
use std::sync::Arc;

use super::{AmbientContractCollector, AmbientContractConfig, ResolvedAmbientContracts};
use crate::ShellDialect;

fn contract_for(path: &Path, source: &str) -> Option<FileContract> {
    contract_for_shell(path, source, ShellDialect::Sh)
}

fn contract_for_shell(path: &Path, source: &str, shell: ShellDialect) -> Option<FileContract> {
    contract_for_optional_path(Some(path), source, shell)
}

fn contract_for_optional_path(
    path: Option<&Path>,
    source: &str,
    shell: ShellDialect,
) -> Option<FileContract> {
    contract_for_optional_path_with_contracts(
        path,
        source,
        shell,
        Arc::new(ResolvedAmbientContracts::default()),
    )
}

fn contract_for_optional_path_with_contracts(
    path: Option<&Path>,
    source: &str,
    shell: ShellDialect,
    contracts: Arc<ResolvedAmbientContracts>,
) -> Option<FileContract> {
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let mut observer = NoopTraversalObserver;
    let mut collector = AmbientContractCollector::new(source, path, shell, contracts);
    let _semantic = build_with_observer_with_options(
        &output.file,
        source,
        &indexer,
        &mut observer,
        SemanticBuildOptions {
            file_entry_contract_collector: Some(&mut collector),
            ..SemanticBuildOptions::default()
        },
    );
    collector.finish()
}

fn request_contract_for(path: &Path, request: PluginRequest) -> FileContract {
    ResolvedAmbientContracts::default()
        .request_contracts_for_plugin(path, &request)
        .requesting_file_contract
}

fn has_initialized_binding(contract: &FileContract, name: &str) -> bool {
    contract.provided_bindings.iter().any(|binding| {
        binding.name == name
            && binding.file_entry_initialization
                == shuck_semantic::FileEntryBindingInitialization::Initialized
    })
}

fn has_ambient_binding(contract: &FileContract, name: &str) -> bool {
    contract.provided_bindings.iter().any(|binding| {
        binding.name == name
            && binding.file_entry_initialization
                == shuck_semantic::FileEntryBindingInitialization::AmbientOnly
    })
}

fn has_consumed_prefix(contract: &FileContract, prefix: &str) -> bool {
    contract
        .externally_consumed_binding_prefixes
        .iter()
        .any(|consumed_prefix| consumed_prefix.as_str() == prefix)
}

fn has_consumed_name(contract: &FileContract, name: &str) -> bool {
    contract
        .externally_consumed_binding_names
        .iter()
        .any(|consumed_name| consumed_name.as_str() == name)
}

fn plugin_request(framework: PluginFramework, name: &str) -> PluginRequest {
    PluginRequest {
        framework,
        kind: PluginRequestKind::Plugin,
        name: name.to_owned(),
        span: shuck_ast::Span::new(),
        explicit: false,
        root_hint: None,
    }
}

#[test]
fn project_specific_paths_do_not_get_ambient_contracts() {
    let void_path = Path::new("/tmp/void-packages/common/build-style/void-cross.sh");
    let void_source = "\
helper() { cd \"${wrksrc}\"; }
printf '%s\\n' \"$pkgname\" \"$pkgver\" \"$XBPS_SRCPKGDIR\" \"$configure_args\"
";
    assert!(contract_for(void_path, void_source).is_none());

    let flattened_path =
        Path::new("/tmp/scripts/void-linux__void-packages__common__build-style__void-cross.sh");
    let flattened_source = "\
helper() { :; }
printf '%s\\n' \"$XBPS_SRCPKGDIR\" \"$configure_args\" \"$wrksrc\"
";
    assert!(contract_for(flattened_path, flattened_source).is_none());
}

#[test]
fn effective_well_known_ids_include_registry_backed_bash_and_zsh_array_contracts() {
    let effective = ResolvedAmbientContracts::default().effective();

    assert!(
        effective
            .well_known_ids
            .iter()
            .any(|id| id.starts_with("bash/"))
    );
    assert!(
        effective
            .well_known_ids
            .iter()
            .any(|id| id.starts_with("bash-it/"))
    );
    assert!(
        effective
            .well_known_ids
            .contains(&"zsh/caller-scoped-arrays".to_owned())
    );
}

#[test]
fn bash_it_theme_paths_get_palette_ambient_contracts() {
    let path = Path::new("/tmp/Bash-it/themes/example/example.theme.bash");
    let source = "\
prompt_command() {
  PS1=\"${green?} ${green} ${reset_color?}\"
}
PROMPT_COMMAND=prompt_command
";

    let contract = contract_for(path, source).unwrap();

    assert!(has_ambient_binding(&contract, "green"));
    assert!(has_ambient_binding(&contract, "reset_color"));
    assert!(!has_initialized_binding(&contract, "green"));
    assert!(!has_initialized_binding(&contract, "reset_color"));
    assert!(!contract.externally_consumed_bindings);
}

#[test]
fn disabling_bash_it_theme_selector_removes_palette_bindings() {
    let path = Path::new("/tmp/Bash-it/themes/example/example.theme.bash");
    let source = "\
prompt_command() {
  PS1=\"${green?} ${green} ${reset_color?}\"
}
PROMPT_COMMAND=prompt_command
";
    let contracts = Arc::new(
        ResolvedAmbientContracts::resolve(
            "/tmp",
            AmbientContractConfig {
                disabled: vec!["bash-it/theme".to_owned()],
                ..AmbientContractConfig::default()
            },
        )
        .unwrap(),
    );

    let contract =
        contract_for_optional_path_with_contracts(Some(path), source, ShellDialect::Sh, contracts);

    assert!(contract.is_none(), "{contract:?}");
}

#[test]
fn generic_theme_paths_do_not_initialize_palette_contracts() {
    let path = Path::new("/tmp/project/themes/example.theme.bash");
    let source = "\
helper() {
  printf '%s\\n' \"$green\" \"$reset_color\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn zsh_config_namespaces_get_consumed_prefix_contracts() {
    let path = Path::new("/tmp/zdot/.zshrc");
    let source = "\
POWERLEVEL9K_LEFT_PROMPT_ELEMENTS=(dir vcs)
ZDOT_MODULE_NAME=prompt
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_consumed_prefix(&contract, "POWERLEVEL9K_"));
    assert!(has_consumed_prefix(&contract, "ZDOT_"));
    assert!(!contract.externally_consumed_bindings);
}

#[test]
fn zsh_runtime_contract_initializes_special_parameters_and_prompt_colors() {
    let path = Path::new("/tmp/zsh/ohmyzsh/plugins/example/example.plugin.zsh");
    let source = "\
print -r -- \"$sysparams\" \"$history\" \"$words\" \"$compstate\" \"$userdirs\"
prompt=\"%{$fg_bold[blue]%}%{$reset_color%}\"
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in [
        "sysparams",
        "history",
        "words",
        "compstate",
        "userdirs",
        "fg_bold",
        "reset_color",
    ] {
        assert!(has_initialized_binding(&contract, name), "{contract:?}");
    }
}

#[test]
fn powerlevel10k_internal_paths_get_runtime_special_parameters() {
    let path = Path::new("/tmp/zsh/powerlevel10k/internal/parser.zsh");
    let source = "print -r -- \"$galiases\" \"$saliases\" \"$langinfo\" \"$sysparams\"\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["galiases", "saliases", "langinfo", "sysparams"] {
        assert!(has_initialized_binding(&contract, name), "{contract:?}");
    }
}

#[test]
fn zmodload_parameter_contract_includes_patch_character_arrays() {
    let path = Path::new("/tmp/zsh/powerlevel10k/gitstatus/mbuild");
    let source = "\
zmodload zsh/parameter zsh/param/private || return
print -r -- \"$patchars\" \"$dis_patchars\"
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["patchars", "dis_patchars"] {
        assert!(has_initialized_binding(&contract, name), "{contract:?}");
    }
}

#[test]
fn powerlevel10k_internal_paths_get_hook_arrays() {
    let path = Path::new("/tmp/zsh/powerlevel10k/internal/p10k.zsh");
    let source = "print -r -- $zsh_directory_name_functions\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_initialized_binding(
        &contract,
        "zsh_directory_name_functions"
    ));
}

#[test]
fn standalone_zsh_scripts_do_not_get_userdirs_without_runtime_context() {
    let path = Path::new("/tmp/project/scripts/example.zsh");
    let source = "print -r -- \"$userdirs\"\n";

    assert!(contract_for_shell(path, source, ShellDialect::Zsh).is_none());
}

#[test]
fn commented_zmodload_does_not_initialize_module_parameters() {
    let path = Path::new("/tmp/project/scripts/example.zsh");
    let source = "\
true;# zmodload zsh/parameter
print -r -- \"$history\"
";

    assert!(contract_for_shell(path, source, ShellDialect::Zsh).is_none());
}

#[test]
fn zsh_runtime_contract_initializes_hook_arrays() {
    let path = Path::new("/tmp/zsh/ohmyzsh/lib/async_prompt.zsh");
    let source = "\
precmd_functions=(${precmd_functions:#_async_prompt_precmd})
print -r -- $chpwd_functions
chpwd_functions+=(_async_prompt_chpwd)
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["precmd_functions", "chpwd_functions"] {
        assert!(has_initialized_binding(&contract, name), "{contract:?}");
        assert!(has_consumed_name(&contract, name), "{contract:?}");
    }
}

#[test]
fn oh_my_zsh_theme_and_appearance_consumes_theme_prefixes() {
    let path = Path::new("/tmp/zsh/ohmyzsh/lib/theme-and-appearance.zsh");
    let source = "ZSH_THEME_GIT_PROMPT_PREFIX='git:('\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_consumed_prefix(&contract, "ZSH_THEME_"));
}

#[test]
fn oh_my_zsh_adben_theme_consumes_theme_prefixes() {
    let path = Path::new("/tmp/zsh/ohmyzsh/themes/adben.zsh-theme");
    let source = "ZSH_THEME_GIT_PROMPT_PREFIX='git:('\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_consumed_prefix(&contract, "ZSH_THEME_"));
}

#[test]
fn oh_my_zsh_sonicradish_theme_consumes_theme_prefixes() {
    let path = Path::new("/tmp/zsh/ohmyzsh/themes/sonicradish.zsh-theme");
    let source = "ZSH_THEME_GIT_PROMPT_PREFIX='git:('\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_consumed_prefix(&contract, "ZSH_THEME_"));
}

#[test]
fn oh_my_zsh_refined_theme_provides_vcs_info() {
    let path = Path::new("/tmp/zsh/ohmyzsh/themes/refined.zsh-theme");
    let source = "print -r -- \"$vcs_info_msg_0_\"\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_ambient_binding(&contract, "vcs_info_msg_0_"));
}

#[test]
fn oh_my_zsh_core_paths_get_plugin_configuration_and_vcs_hook_state() {
    let core_path = Path::new("/tmp/zsh/ohmyzsh/oh-my-zsh.sh");
    let core_source = "print -r -- \"$plugins\"\n";
    let core_contract = contract_for_shell(core_path, core_source, ShellDialect::Zsh).unwrap();
    assert!(has_initialized_binding(&core_contract, "plugins"));

    let git_path = Path::new("/tmp/zsh/ohmyzsh/lib/git.zsh");
    let git_source = "print -r -- \"${hook_com[branch]}\"\n";
    let git_contract = contract_for_shell(git_path, git_source, ShellDialect::Zsh).unwrap();
    assert!(has_ambient_binding(&git_contract, "hook_com"));
}

#[test]
fn oh_my_zsh_emoji_and_emotty_paths_get_emoji_runtime_bindings() {
    let emoji_path = Path::new("/tmp/zsh/ohmyzsh/plugins/emoji/emoji.plugin.zsh");
    let emoji_source = "print -r -- \"$emoji_groups[people]\" \"$emoji_flags[us]\" \"$emoji[rocket]\"\n";
    let emoji_contract = contract_for_shell(emoji_path, emoji_source, ShellDialect::Zsh).unwrap();
    for name in ["emoji", "emoji_flags", "emoji_groups"] {
        assert!(has_initialized_binding(&emoji_contract, name), "{emoji_contract:?}");
    }

    let emotty_path = Path::new("/tmp/zsh/ohmyzsh/plugins/emotty/emotty.plugin.zsh");
    let emotty_source = "print -r -- \"$emoji[rocket]\" \"$emoji2[emoji_style]\"\n";
    let emotty_contract =
        contract_for_shell(emotty_path, emotty_source, ShellDialect::Zsh).unwrap();
    for name in ["emoji", "emoji2"] {
        assert!(
            has_initialized_binding(&emotty_contract, name),
            "{emotty_contract:?}"
        );
    }

    let theme_path = Path::new("/tmp/zsh/ohmyzsh/themes/emotty.zsh-theme");
    let theme_source = "print -r -- \"$emoji[skull]\" \"$emoji2[emoji_style]\"\n";
    let theme_contract = contract_for_shell(theme_path, theme_source, ShellDialect::Zsh).unwrap();
    for name in ["emoji", "emoji2"] {
        assert!(
            has_initialized_binding(&theme_contract, name),
            "{theme_contract:?}"
        );
    }
}

#[test]
fn oh_my_zsh_tools_get_prompt_color_bindings() {
    let path = Path::new("/tmp/zsh/ohmyzsh/tools/check_for_upgrade.sh");
    let source = "print -r -- \"$fg\" \"$reset_color\"\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_initialized_binding(&contract, "fg"));
    assert!(has_initialized_binding(&contract, "reset_color"));
}

#[test]
fn hyphenated_oh_my_zsh_paths_get_unknown_shell_runtime_contracts() {
    let path = Path::new("/tmp/home/.oh-my-zsh/tools/check_for_upgrade.sh");
    let source = "print -r -- \"$fg\" \"$reset_color\"\n";

    let contract = contract_for_shell(path, source, ShellDialect::Unknown).unwrap();

    assert!(has_initialized_binding(&contract, "fg"));
    assert!(has_initialized_binding(&contract, "reset_color"));
}

#[test]
fn zsh_runtime_contract_initializes_braced_prompt_color_arrays() {
    let path = Path::new("/tmp/zsh/ohmyzsh/plugins/example/example.plugin.zsh");
    let source = "\
typeset -AHg less_termcap
less_termcap[mb]=\"${fg_bold[red]}\"
less_termcap[so]=\"${fg_bold[yellow]}${bg[blue]}\"
less_termcap[me]=\"${reset_color}\"
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["fg_bold", "bg", "reset_color"] {
        assert!(has_initialized_binding(&contract, name), "{contract:?}");
    }
}

#[test]
fn zsh_runtime_contract_initializes_prompt_colors_after_colors_autoload() {
    let path = Path::new("/tmp/zsh/holman-dotfiles/zsh/prompt.zsh");
    let source = "\
autoload colors && colors
echo \"on %{$fg_bold[green]%}%{$reset_color%}\"
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["fg_bold", "reset_color"] {
        assert!(has_initialized_binding(&contract, name), "{contract:?}");
    }
}

#[test]
fn zsh_runtime_contract_accepts_compact_colors_autoload_separators() {
    let path = Path::new("/tmp/zsh/holman-dotfiles/zsh/prompt.zsh");
    let source = "\
autoload colors&&colors
echo \"on %{$fg_bold[green]%}%{$reset_color%}\"
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["fg_bold", "reset_color"] {
        assert!(has_initialized_binding(&contract, name), "{contract:?}");
    }
}

#[test]
fn zsh_runtime_contract_requires_colors_autoload_command() {
    let path = Path::new("/tmp/zsh/holman-dotfiles/zsh/prompt.zsh");
    let source = "\
echo autoload colors
echo \"on %{$fg_bold[green]%}%{$reset_color%}\"
";

    assert!(contract_for_shell(path, source, ShellDialect::Zsh).is_none());
}

#[test]
fn zsh_history_state_assignments_are_consumed_in_config_contexts() {
    let path = Path::new("/tmp/home/zsh/config.zsh");
    let source = "\
HISTFILE=~/.zsh_history
HISTSIZE=10000
SAVEHIST=10000
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["HISTFILE", "HISTSIZE", "SAVEHIST"] {
        assert!(has_consumed_name(&contract, name), "{contract:?}");
    }
}

#[test]
fn zsh_module_metadata_triplets_are_consumed_by_external_loaders() {
    let path = Path::new("/tmp/project/install/01-package-manager.zsh");
    let source = "\
#!/bin/zsh
module_name=\"package-manager\"
module_description=\"Install package manager\"
module_main_function=\"run_package_manager_module\"

run_package_manager_module() {
  :
}
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["module_name", "module_description", "module_main_function"] {
        assert!(has_consumed_name(&contract, name), "{contract:?}");
    }
}

#[test]
fn zsh_module_metadata_accepts_next_line_function_body_brace() {
    let path = Path::new("/tmp/project/install/01-package-manager.zsh");
    let source = "\
#!/bin/zsh
module_name=\"package-manager\"
module_description=\"Install package manager\"
module_main_function=\"run_package_manager_module\"

function run_package_manager_module
{
  :
}
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["module_name", "module_description", "module_main_function"] {
        assert!(has_consumed_name(&contract, name), "{contract:?}");
    }
}

#[test]
fn zsh_module_metadata_accepts_name_paren_next_line_function_body_brace() {
    let path = Path::new("/tmp/project/install/01-package-manager.zsh");
    let source = "\
#!/bin/zsh
module_name=\"package-manager\"
module_description=\"Install package manager\"
module_main_function=\"run_package_manager_module\"

run_package_manager_module()
{
  :
}
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["module_name", "module_description", "module_main_function"] {
        assert!(has_consumed_name(&contract, name), "{contract:?}");
    }
}

#[test]
fn zsh_module_metadata_requires_exact_main_function_name() {
    let path = Path::new("/tmp/project/install/01-package-manager.zsh");
    let source = "\
#!/bin/zsh
module_name=\"package-manager\"
module_description=\"Install package manager\"
module_main_function=\"run\"

runner() {
  :
}
";

    assert!(contract_for_shell(path, source, ShellDialect::Zsh).is_none());
}

#[test]
fn zsh_module_metadata_rejects_command_before_brace_group() {
    let path = Path::new("/tmp/project/install/01-package-manager.zsh");
    let source = "\
#!/bin/zsh
module_name=\"package-manager\"
module_description=\"Install package manager\"
module_main_function=\"run\"

run
{
  print -r -- not-a-function
}
";

    assert!(contract_for_shell(path, source, ShellDialect::Zsh).is_none());
}

#[test]
fn unknown_generic_runtime_paths_do_not_get_zsh_runtime_contracts() {
    let path = Path::new("/tmp/project/plugins/example");
    let source = "print -r -- \"$history\" \"$words\"\n";

    assert!(contract_for_shell(path, source, ShellDialect::Unknown).is_none());
}

#[test]
fn pathless_zsh_sources_do_not_get_ambient_runtime_contracts() {
    let source = "print -r -- \"$history\" \"$words\" \"$fg_bold\"\n";

    assert!(contract_for_optional_path(None, source, ShellDialect::Zsh).is_none());
}

#[test]
fn zsh_runtime_contract_marks_exact_output_parameters_consumed() {
    let path = Path::new("/tmp/zsh/ohmyzsh/plugins/example/example.plugin.zsh");
    let source = "\
reply=(one two)
REPLY=value
WORDCHARS=''
compstate[insert]=menu
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["reply", "REPLY", "WORDCHARS", "compstate"] {
        assert!(has_consumed_name(&contract, name), "{contract:?}");
    }
}

#[test]
fn pathless_zsh_hook_arrays_do_not_get_ambient_contracts() {
    let source = "precmd_functions=(${precmd_functions:#_example_precmd})\n";

    assert!(contract_for_optional_path(None, source, ShellDialect::Zsh).is_none());
}

#[test]
fn zsh_runtime_contract_does_not_consume_read_only_output_parameters() {
    let path = Path::new("/tmp/zsh/ohmyzsh/plugins/example/example.plugin.zsh");
    let source = "print -r -- \"$reply\" \"$REPLY\" \"$compstate\"\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["reply", "REPLY", "compstate"] {
        assert!(!has_consumed_name(&contract, name), "{contract:?}");
    }
}

#[test]
fn powerlevel10k_internal_bootstrap_files_initialize_injected_bindings() {
    let cases = [
        (
            "/tmp/zsh/powerlevel10k/internal/p10k.zsh",
            "[[ $__p9k_sourced == 1 ]] || return\neval \"$__p9k_intro\"\nprint -r -- \"$__p9k_root_dir\"\n",
            &["__p9k_sourced", "__p9k_root_dir", "__p9k_intro"][..],
        ),
        (
            "/tmp/zsh/powerlevel10k/internal/configure.zsh",
            "print -r -- \"$__p9k_root_dir\"\neval \"$__p9k_intro\"\n",
            &["__p9k_root_dir", "__p9k_intro"][..],
        ),
        (
            "/tmp/zsh/powerlevel10k/internal/wizard.zsh",
            "print -r -- \"$__p9k_root_dir\"\neval \"$__p9k_intro\"\n",
            &["__p9k_root_dir", "__p9k_intro"][..],
        ),
    ];

    for (path, source, expected_bindings) in cases {
        let contract = contract_for_shell(Path::new(path), source, ShellDialect::Zsh).unwrap();
        for name in expected_bindings {
            assert!(has_initialized_binding(&contract, name), "{contract:?}");
        }
    }
}

#[test]
fn powerlevel10k_internal_paths_consume_public_and_private_config_prefixes() {
    let path = Path::new("/tmp/zsh/powerlevel10k/internal/p10k.zsh");
    let source = "\
print -r -- \"$POWERLEVEL9K_LEFT_PROMPT_ELEMENTS\"
print -r -- \"$_POWERLEVEL9K_RBENV_SOURCES\"
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for prefix in ["POWERLEVEL9K_", "_POWERLEVEL9K_"] {
        assert!(has_consumed_prefix(&contract, prefix), "{contract:?}");
    }
}

#[test]
fn powerlevel10k_internal_paths_get_configure_state_bindings() {
    let path = Path::new("/tmp/zsh/powerlevel10k/internal/wizard.zsh");
    let source = "\
print -r -- \"$__p9k_cfg_path\" \"$__p9k_cfg_path_u\" \"$__p9k_zshrc\" \"$__p9k_zd\" \"$__p9k_wizard_columns\"
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in [
        "__p9k_cfg_path",
        "__p9k_cfg_path_u",
        "__p9k_zshrc",
        "__p9k_zd",
        "__p9k_wizard_columns",
    ] {
        assert!(has_initialized_binding(&contract, name), "{contract:?}");
    }
}

#[test]
fn powerlevel10k_wizard_paths_get_shared_ui_state_bindings() {
    let wizard_path = Path::new("/tmp/zsh/powerlevel10k/internal/wizard.zsh");
    let wizard_source = "print -r -- \"$icons[VCS_GIT_ICON]\"; (( _p9k_term_has_href ))\n";
    let wizard_contract =
        contract_for_shell(wizard_path, wizard_source, ShellDialect::Zsh).unwrap();
    for name in ["icons", "_p9k_term_has_href"] {
        assert!(has_initialized_binding(&wizard_contract, name), "{wizard_contract:?}");
    }
}

#[test]
fn powerlevel10k_p10k_paths_get_instant_prompt_and_vcs_info_bindings() {
    let path = Path::new("/tmp/zsh/powerlevel10k/internal/p10k.zsh");
    let source = "\
vcs_info
print -r -- \"$vcs_info_msg_0_\" \"$__p9k_intro_no_locale\" \"$__p9k_dump_file\" \"$__p9k_instant_prompt_time\"
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in [
        "vcs_info_msg_0_",
        "__p9k_intro_no_locale",
        "__p9k_dump_file",
        "__p9k_instant_prompt_time",
    ] {
        assert!(has_initialized_binding(&contract, name), "{contract:?}");
    }
}

#[test]
fn powerlevel10k_gitstatus_zsh_contract_initializes_intro_base_and_consumes_status_prefix() {
    let path = Path::new("/tmp/zsh/powerlevel10k/gitstatus/gitstatus.plugin.zsh");
    let source = "\
eval \"$__p9k_intro_base\"
typeset -g VCS_STATUS_RESULT=ok
typeset -g VCS_STATUS_HAS_STAGED=1
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_initialized_binding(&contract, "__p9k_intro_base"));
    assert!(has_consumed_prefix(&contract, "VCS_STATUS_"));
}

#[test]
fn powerlevel10k_gitstatus_sh_contract_consumes_status_prefix() {
    let path = Path::new("/tmp/zsh/powerlevel10k/gitstatus/gitstatus.plugin.sh");
    let source = "\
VCS_STATUS_RESULT=ok
VCS_STATUS_HAS_STAGED=1
";

    let contract = contract_for_shell(path, source, ShellDialect::Sh).unwrap();

    assert!(has_consumed_prefix(&contract, "VCS_STATUS_"));
    assert!(!has_initialized_binding(&contract, "__p9k_intro_base"));
}

#[test]
fn zsh_syntax_highlighting_test_data_expected_values_are_consumed() {
    let path =
        Path::new("/tmp/zsh/zsh-syntax-highlighting/highlighters/main/test-data/example.zsh");
    let source = "expected_region_highlight=('1 4 fg=red')\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_consumed_prefix(&contract, "expected_"));
}

#[test]
fn zsh_syntax_highlighting_test_harness_expected_values_are_consumed() {
    let path = Path::new("/tmp/zsh/zsh-syntax-highlighting/tests/test-zprof.zsh");
    let source = "\
run_test_internal() {
  expected_region_highlight=()
  true && _zsh_highlight
}
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_consumed_prefix(&contract, "expected_"));
}

#[test]
fn zsh_autosuggestion_config_namespace_is_consumed_on_project_paths() {
    let path = Path::new("/tmp/zsh/zsh-autosuggestions/src/config.zsh");
    let source = "ZSH_AUTOSUGGEST_STRATEGY=(history)\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_consumed_prefix(&contract, "ZSH_AUTOSUGGEST_"));
}

#[test]
fn prezto_autosuggestions_module_paths_consume_config_prefix() {
    let path = Path::new("/tmp/zsh/prezto/modules/autosuggestions/init.zsh");
    let source = "ZSH_AUTOSUGGEST_HIGHLIGHT_STYLE='fg=8'\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_consumed_prefix(&contract, "ZSH_AUTOSUGGEST_"));
}

#[test]
fn zsh_plugin_request_contracts_consume_known_framework_config_prefixes() {
    let path = Path::new("/tmp/zdot/modules/autocompletion/autocompletion.zsh");

    let autosuggest = request_contract_for(
        path,
        plugin_request(
            PluginFramework::Other("zsh-autosuggestions".to_owned()),
            "zsh-autosuggestions",
        ),
    );
    assert!(has_consumed_prefix(&autosuggest, "ZSH_AUTOSUGGEST_"));

    let abbr = request_contract_for(
        path,
        plugin_request(PluginFramework::Other("zsh-abbr".to_owned()), "zsh-abbr"),
    );
    assert!(has_consumed_prefix(&abbr, "ABBR_"));

    let highlighting = request_contract_for(
        path,
        plugin_request(
            PluginFramework::Other("zsh-syntax-highlighting".to_owned()),
            "zsh-syntax-highlighting",
        ),
    );
    assert!(has_consumed_prefix(&highlighting, "ZSH_HIGHLIGHT_"));
}

#[test]
fn zsh_syntax_highlighting_highlighter_paths_inherit_user_options() {
    let path = Path::new("/tmp/zsh/zsh-syntax-highlighting/highlighters/main/main-highlighter.zsh");
    let source = "print -r -- \"$zsyh_user_options[ignorebraces]\"\n";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_initialized_binding(&contract, "zsyh_user_options"));
}

#[test]
fn zsh_syntax_highlighting_test_paths_consume_harness_state() {
    let path = Path::new("/tmp/zsh/zsh-syntax-highlighting/tests/test-highlighting.zsh");
    let source = "\
PREBUFFER=''
MARK=1
REGION_ACTIVE=2
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["PREBUFFER", "MARK", "REGION_ACTIVE"] {
        assert!(has_consumed_name(&contract, name), "{contract:?}");
    }
}

#[test]
fn zsh_config_prefix_contracts_are_zsh_only() {
    let path = Path::new("/tmp/project/script.sh");
    let source = "POWERLEVEL9K_DIR_FOREGROUND=31\n";

    assert!(contract_for_shell(path, source, ShellDialect::Sh).is_none());
}

#[test]
fn ordinary_zsh_paths_do_not_get_config_prefix_contracts() {
    let path = Path::new("/tmp/project/plugins/example.zsh");
    let source = "\
POWERLEVEL9K_DIR_FOREGROUND=31
ZDOT_MODULE_NAME=prompt
";

    assert!(contract_for_shell(path, source, ShellDialect::Zsh).is_none());
}

#[test]
fn shebangless_zsh_dotfiles_get_config_prefix_contracts() {
    let path = Path::new("/tmp/home/.zshrc");
    let source = "\
POWERLEVEL9K_DIR_FOREGROUND=31
ZDOT_MODULE_NAME=prompt
";

    let contract = contract_for_shell(path, source, ShellDialect::Unknown).unwrap();

    assert!(has_consumed_prefix(&contract, "POWERLEVEL9K_"));
    assert!(has_consumed_prefix(&contract, "ZDOT_"));
}

#[test]
fn zshrc_named_directories_do_not_count_as_dotfiles() {
    let path = Path::new("/tmp/project/zshrc-theme/prompt.zsh");
    let source = "\
POWERLEVEL9K_DIR_FOREGROUND=31
ZDOT_MODULE_NAME=prompt
";

    assert!(contract_for_shell(path, source, ShellDialect::Zsh).is_none());
}

#[test]
fn unknown_zshrc_backup_paths_do_not_get_runtime_contracts() {
    let path = Path::new("/tmp/project/zshrc_backup/plugins/example.zsh");
    let source = "print -r -- \"$history\"\n";

    assert!(contract_for_shell(path, source, ShellDialect::Unknown).is_none());
}

#[test]
fn zsh_sourced_helpers_initialize_caller_scoped_array_length_targets() {
    let path = Path::new("/tmp/project/core/update_core.zsh");
    let source = "\
#!/bin/zsh
safe_rm() {
  if [[ ${#dry_run[@]} -gt 0 ]]; then
    print -r -- dry
  fi
}
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(has_initialized_binding(&contract, "dry_run"));
    assert!(!contract.externally_consumed_bindings);
}

#[test]
fn disabling_zsh_caller_scoped_array_contract_selector_keeps_helper_array_reads_reportable() {
    let path = Path::new("/tmp/project/core/update_core.zsh");
    let source = "\
#!/bin/zsh
safe_rm() {
  if [[ ${#dry_run[@]} -gt 0 ]]; then
    print -r -- dry
  fi
}
";
    let contracts = Arc::new(
        ResolvedAmbientContracts::resolve(
            "/tmp/project",
            AmbientContractConfig {
                disabled: vec!["zsh/caller-scoped-arrays".to_owned()],
                ..AmbientContractConfig::default()
            },
        )
        .unwrap(),
    );

    let contract =
        contract_for_optional_path_with_contracts(Some(path), source, ShellDialect::Zsh, contracts);

    assert!(contract.is_none(), "{contract:?}");
}

#[test]
fn zsh_sourced_helper_array_contracts_ignore_comments() {
    let path = Path::new("/tmp/project/core/update_core.zsh");
    let source = "\
#!/bin/zsh
# if [[ ${#dry_run[@]} -gt 0 ]]; then
safe_rm() {
  print -r -- $dry_run
}
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh);

    assert!(contract.is_none());
}

#[test]
fn generic_completion_paths_do_not_initialize_completion_contracts() {
    let path = Path::new("/tmp/project/completions/example.sh");
    let source = "\
helper() {
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn generic_completion_paths_with_compreply_do_not_initialize_completion_contracts() {
    let path = Path::new("/tmp/project/completions/example.sh");
    let source = "\
helper() {
  COMPREPLY=()
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn bash_completion_paths_without_initializer_do_not_initialize_completion_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
_example() {
  COMPREPLY=()
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn bash_completion_paths_with_initializer_get_ambient_completion_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
_example() {
  _init_completion || return
  printf '%s\\n' \"$cur\" \"$cword\" \"$comp_args\"
}
";

    let contract = contract_for(path, source).unwrap();

    assert!(has_ambient_binding(&contract, "cur"));
    assert!(has_ambient_binding(&contract, "cword"));
    assert!(has_ambient_binding(&contract, "comp_args"));
    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!has_initialized_binding(&contract, "comp_args"));
    assert!(!contract.externally_consumed_bindings);
}

#[test]
fn bash_completion_paths_with_chained_initializer_wrappers_get_ambient_completion_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
_example() {
  command env LC_ALL=C _init_completion || return
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    let contract = contract_for(path, source).unwrap();

    assert!(has_ambient_binding(&contract, "cur"));
    assert!(has_ambient_binding(&contract, "cword"));
    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
}

#[test]
fn bash_completion_paths_with_env_then_shell_wrapper_get_ambient_completion_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
_example() {
  env LC_ALL=C command _init_completion || return
  env LC_ALL=\"$locale\" _get_comp_words_by_ref cur || return
  env LC_ALL=C env _comp_initialize || return
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    let contract = contract_for(path, source).unwrap();

    assert!(has_ambient_binding(&contract, "cur"));
    assert!(has_ambient_binding(&contract, "cword"));
    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
}

#[test]
fn bash_completion_paths_with_external_initializer_wrappers_do_not_initialize_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
_example() {
  sudo _init_completion || return
  sudo env _init_completion || return
  env LC_ALL=C sudo _get_comp_words_by_ref || return
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn bash_completion_paths_with_subshell_initializer_do_not_initialize_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
_example() {
  printf '%s\\n' \"$(_init_completion)\" \"$cur\" \"$cword\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn bash_completion_paths_with_commented_initializer_do_not_initialize_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
# TODO: call _init_completion later
_example() {
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn bash_completion_paths_with_wrapper_identifier_do_not_initialize_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
_example() {
  my_init_completion_wrapper || return
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn bash_completion_paths_with_initializer_definition_do_not_initialize_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
_init_completion() {
  :
}
_example() {
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn bash_completion_paths_with_separator_comment_do_not_initialize_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
noop;# _init_completion later
_example() {
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn bash_completion_paths_with_initializer_in_heredoc_do_not_initialize_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
cat <<EOF
_init_completion
EOF
_example() {
  printf '%s\\n' \"$cur\" \"$cword\"
}
";

    assert!(contract_for(path, source).is_none());
}

#[test]
fn sourced_runtime_module_paths_do_not_initialize_arbitrary_reads() {
    let path = Path::new("/tmp/LinuxGSM/lgsm/modules/command_backup.sh");
    let source = "\
commandname=\"BACKUP\"
backup_run() {
  printf '%s\\n' \"$lockdir\" \"$commandname\"
}
";

    assert!(contract_for(path, source).is_none());
}
