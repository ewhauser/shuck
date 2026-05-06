use std::path::Path;

use shuck_indexer::Indexer;
use shuck_parser::parser::Parser;
use shuck_semantic::{
    FileContract, FileEntryContractCollector, NoopTraversalObserver, SemanticBuildOptions,
    build_with_observer_with_options,
};

use super::AmbientContractCollector;
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
    let output = Parser::new(source).parse().unwrap();
    let indexer = Indexer::new(source, &output);
    let mut observer = NoopTraversalObserver;
    let mut collector = AmbientContractCollector::new(source, path, shell);
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
fn generic_theme_paths_do_not_initialize_palette_contracts() {
    let path = Path::new("/tmp/project/themes/example.theme.bash");
    let source = "\
helper() {
  printf '%s\\n' \"$green\" \"$reset_color\"
}
";

    let contract = contract_for(path, source).unwrap();

    assert!(!has_initialized_binding(&contract, "green"));
    assert!(!has_initialized_binding(&contract, "reset_color"));
    assert!(!contract.externally_consumed_bindings);
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
print -r -- \"$sysparams\" \"$history\" \"$words\" \"$compstate\"
prompt=\"%{$fg_bold[blue]%}%{$reset_color%}\"
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in [
        "sysparams",
        "history",
        "words",
        "compstate",
        "fg_bold",
        "reset_color",
    ] {
        assert!(has_initialized_binding(&contract, name), "{contract:?}");
    }
}

#[test]
fn commented_zmodload_does_not_initialize_module_parameters() {
    let path = Path::new("/tmp/project/scripts/example.zsh");
    let source = "\
true;# zmodload zsh/parameter
print -r -- \"$history\"
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    assert!(!has_initialized_binding(&contract, "history"));
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
compstate[insert]=menu
TIMEFMT='%E'
";

    let contract = contract_for_shell(path, source, ShellDialect::Zsh).unwrap();

    for name in ["reply", "REPLY", "compstate", "TIMEFMT"] {
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

    let contract = contract_for(path, source).unwrap();

    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
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

    let contract = contract_for(path, source).unwrap();

    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
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

    let contract = contract_for(path, source).unwrap();

    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
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

    let contract = contract_for(path, source).unwrap();

    assert!(!has_ambient_binding(&contract, "cur"));
    assert!(!has_ambient_binding(&contract, "cword"));
    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
}

#[test]
fn bash_completion_paths_with_subshell_initializer_do_not_initialize_contracts() {
    let path = Path::new("/tmp/bash-completion/completions/example.bash");
    let source = "\
_example() {
  printf '%s\\n' \"$(_init_completion)\" \"$cur\" \"$cword\"
}
";

    let contract = contract_for(path, source).unwrap();

    assert!(!has_ambient_binding(&contract, "cur"));
    assert!(!has_ambient_binding(&contract, "cword"));
    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
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

    let contract = contract_for(path, source).unwrap();

    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
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

    let contract = contract_for(path, source).unwrap();

    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
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

    let contract = contract_for(path, source).unwrap();

    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
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

    let contract = contract_for(path, source).unwrap();

    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
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

    let contract = contract_for(path, source).unwrap();

    assert!(!has_initialized_binding(&contract, "cur"));
    assert!(!has_initialized_binding(&contract, "cword"));
    assert!(!contract.externally_consumed_bindings);
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

    let contract = contract_for(path, source).unwrap();

    assert!(!has_initialized_binding(&contract, "lockdir"));
    assert!(!has_initialized_binding(&contract, "commandname"));
    assert!(!contract.externally_consumed_bindings);
}
