use std::collections::BTreeSet;
use std::path::Path;

use shuck_ast::{
    BuiltinCommand, Command, CompoundCommand, File, Name, SimpleCommand,
    StaticCommandWrapperTarget, Stmt, StmtSeq, Word, static_command_name_text,
    static_command_wrapper_target_index, static_word_text,
};
use shuck_semantic::{ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind};

use crate::ShellDialect;

struct AmbientContractProvider {
    matches: fn(file: &File, source: &str, path: &Path, shell: ShellDialect) -> bool,
    build: fn(file: &File, source: &str, path: &Path, shell: ShellDialect) -> FileContract,
}

pub(crate) fn file_entry_contract(
    file: &File,
    source: &str,
    path: Option<&Path>,
    shell: ShellDialect,
) -> Option<FileContract> {
    let path = path?;
    let mut merged = FileContract::default();
    let mut matched = false;

    for provider in providers() {
        if (provider.matches)(file, source, path, shell) {
            matched = true;
            merge_contract(&mut merged, (provider.build)(file, source, path, shell));
        }
    }

    matched.then_some(merged)
}

fn providers() -> &'static [AmbientContractProvider] {
    &[AmbientContractProvider {
        matches: matches_sourced_runtime_contract,
        build: build_sourced_runtime_contract,
    }]
}

fn merge_contract(merged: &mut FileContract, contract: FileContract) {
    merged.externally_consumed_bindings |= contract.externally_consumed_bindings;
    for name in contract.required_reads {
        merged.add_required_read(name);
    }
    for binding in contract.provided_bindings {
        merged.add_provided_binding(binding);
    }
    for function in contract.provided_functions {
        merged.add_provided_function(function);
    }
}

fn matches_sourced_runtime_contract(
    file: &File,
    source: &str,
    path: &Path,
    _shell: ShellDialect,
) -> bool {
    let lower = lower_path(path);
    sourced_runtime_path_shape(&lower) && sourced_runtime_source_shape(file, source, &lower)
}

fn build_sourced_runtime_contract(
    file: &File,
    source: &str,
    path: &Path,
    _shell: ShellDialect,
) -> FileContract {
    let lower = lower_path(path);
    let mut names = BTreeSet::new();

    for name in runtime_names_for_source_path(file, source, &lower) {
        names.insert((*name).to_owned());
    }

    let mut contract = FileContract {
        ..FileContract::default()
    };
    for name in names {
        contract.add_provided_binding(ProvidedBinding::new(
            Name::from(name.as_str()),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }
    contract
}

fn sourced_runtime_path_shape(lower: &str) -> bool {
    path_matches_any(
        lower,
        &[
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
        ],
    )
}

fn sourced_runtime_source_shape(file: &File, source: &str, lower_path: &str) -> bool {
    has_probable_function_definition(source)
        || has_source_command(source)
        || source.contains("PROMPT_COMMAND")
        || source.contains("COMPREPLY")
        || source.contains("about-completion")
        || (lower_path.contains("termux-packages") && source.contains("TERMUX_"))
        || completion_runtime_source_shape(file, source)
}

fn runtime_names_for_source_path(
    file: &File,
    source: &str,
    lower: &str,
) -> &'static [&'static str] {
    if bash_it_theme_runtime_shape(source, lower) {
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

    if completion_runtime_shape(file, source, lower) {
        return &["cur", "prev", "words", "cword", "comp_args", "split"];
    }

    &[]
}

fn bash_it_theme_runtime_shape(source: &str, lower: &str) -> bool {
    path_matches_any(lower, &["/bash-it/themes/", "/bash-it/theme/"])
        && (source.contains("PROMPT_COMMAND")
            || source.contains("SCM_THEME_PROMPT")
            || source_mentions_any(
                source,
                &[
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
                ],
            ))
}

fn completion_runtime_shape(file: &File, source: &str, lower: &str) -> bool {
    completion_runtime_path_shape(lower) && completion_runtime_source_shape(file, source)
}

fn completion_runtime_path_shape(lower: &str) -> bool {
    path_matches_any(
        lower,
        &[
            "/bash-completion/",
            "/bash_completion/",
            "/bash-it/completion/",
            "/bash-it/completions/",
            "/bash-progcomp/",
            "bash_autocomplete",
        ],
    )
}

fn completion_runtime_source_shape(file: &File, source: &str) -> bool {
    stmt_seq_invokes_completion_initializer(&file.body, source)
}

fn stmt_seq_invokes_completion_initializer(sequence: &StmtSeq, source: &str) -> bool {
    sequence
        .iter()
        .any(|stmt| stmt_invokes_completion_initializer(stmt, source))
}

fn stmt_invokes_completion_initializer(stmt: &Stmt, source: &str) -> bool {
    command_invokes_completion_initializer(&stmt.command, source)
}

fn command_invokes_completion_initializer(command: &Command, source: &str) -> bool {
    match command {
        Command::Simple(command) => simple_command_invokes_completion_initializer(command, source),
        Command::Builtin(command) => builtin_invokes_completion_initializer(command, source),
        Command::Decl(_) => false,
        Command::Binary(command) => {
            stmt_invokes_completion_initializer(&command.left, source)
                || stmt_invokes_completion_initializer(&command.right, source)
        }
        Command::Compound(command) => compound_invokes_completion_initializer(command, source),
        Command::Function(function) => stmt_invokes_completion_initializer(&function.body, source),
        Command::AnonymousFunction(function) => {
            stmt_invokes_completion_initializer(&function.body, source)
        }
    }
}

fn simple_command_invokes_completion_initializer(command: &SimpleCommand, source: &str) -> bool {
    let words = std::iter::once(&command.name)
        .chain(command.args.iter())
        .collect::<Vec<_>>();
    word_chain_invokes_completion_initializer(&words, source)
}

fn builtin_invokes_completion_initializer(_command: &BuiltinCommand, _source: &str) -> bool {
    false
}

fn word_chain_invokes_completion_initializer(words: &[&Word], source: &str) -> bool {
    let mut index = 0;

    while let Some(word) = words.get(index) {
        let Some(name) = static_command_name_text(word, source) else {
            return false;
        };

        if is_completion_initializer_command(name.as_ref()) {
            return true;
        }

        if name == "env" {
            let Some(target_index) = env_wrapper_target_index(words, source, index) else {
                return false;
            };
            index = target_index;
            continue;
        }

        match static_command_wrapper_target_index(words.len(), index, name.as_ref(), |index| {
            static_word_text(words[index], source)
        }) {
            StaticCommandWrapperTarget::Wrapper {
                target_index: Some(target_index),
            } => index = target_index,
            StaticCommandWrapperTarget::Wrapper { target_index: None }
            | StaticCommandWrapperTarget::NotWrapper => return false,
        }
    }

    false
}

fn env_wrapper_target_index(words: &[&Word], source: &str, current_index: usize) -> Option<usize> {
    current_index
        .checked_add(1)
        .and_then(|start| words.get(start..))?
        .iter()
        .enumerate()
        .find_map(|(relative_index, word)| {
            let text = static_word_text(word, source)?;
            (!shell_assignment_token(text.as_ref())).then_some(current_index + 1 + relative_index)
        })
}

fn compound_invokes_completion_initializer(command: &CompoundCommand, source: &str) -> bool {
    match command {
        CompoundCommand::If(command) => {
            stmt_seq_invokes_completion_initializer(&command.condition, source)
                || stmt_seq_invokes_completion_initializer(&command.then_branch, source)
                || command.elif_branches.iter().any(|(condition, body)| {
                    stmt_seq_invokes_completion_initializer(condition, source)
                        || stmt_seq_invokes_completion_initializer(body, source)
                })
                || command
                    .else_branch
                    .as_ref()
                    .is_some_and(|branch| stmt_seq_invokes_completion_initializer(branch, source))
        }
        CompoundCommand::For(command) => {
            stmt_seq_invokes_completion_initializer(&command.body, source)
        }
        CompoundCommand::Repeat(command) => {
            stmt_seq_invokes_completion_initializer(&command.body, source)
        }
        CompoundCommand::Foreach(command) => {
            stmt_seq_invokes_completion_initializer(&command.body, source)
        }
        CompoundCommand::ArithmeticFor(command) => {
            stmt_seq_invokes_completion_initializer(&command.body, source)
        }
        CompoundCommand::While(command) => {
            stmt_seq_invokes_completion_initializer(&command.condition, source)
                || stmt_seq_invokes_completion_initializer(&command.body, source)
        }
        CompoundCommand::Until(command) => {
            stmt_seq_invokes_completion_initializer(&command.condition, source)
                || stmt_seq_invokes_completion_initializer(&command.body, source)
        }
        CompoundCommand::Case(command) => command
            .cases
            .iter()
            .any(|case| stmt_seq_invokes_completion_initializer(&case.body, source)),
        CompoundCommand::Select(command) => {
            stmt_seq_invokes_completion_initializer(&command.body, source)
        }
        CompoundCommand::Subshell(commands) | CompoundCommand::BraceGroup(commands) => {
            stmt_seq_invokes_completion_initializer(commands, source)
        }
        CompoundCommand::Arithmetic(_) | CompoundCommand::Conditional(_) => false,
        CompoundCommand::Time(command) => command
            .command
            .as_ref()
            .is_some_and(|stmt| stmt_invokes_completion_initializer(stmt, source)),
        CompoundCommand::Coproc(command) => {
            stmt_invokes_completion_initializer(&command.body, source)
        }
        CompoundCommand::Always(command) => {
            stmt_seq_invokes_completion_initializer(&command.body, source)
                || stmt_seq_invokes_completion_initializer(&command.always_body, source)
        }
    }
}

fn is_completion_initializer_command(token: &str) -> bool {
    matches!(
        token,
        "_init_completion" | "_get_comp_words_by_ref" | "_comp_initialize" | "about-completion"
    )
}

fn shell_assignment_token(token: &str) -> bool {
    let Some((name, _value)) = token.split_once('=') else {
        return false;
    };

    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn lower_path(path: &Path) -> String {
    path.to_string_lossy().to_ascii_lowercase()
}

fn path_matches_any(lower_path: &str, patterns: &[&str]) -> bool {
    patterns.iter().any(|pattern| lower_path.contains(pattern))
}

fn has_probable_function_definition(source: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .any(probable_function_definition)
}

fn has_source_command(source: &str) -> bool {
    source.lines().map(str::trim).any(|trimmed| {
        trimmed.starts_with("source ")
            || trimmed.starts_with(". ")
            || trimmed.starts_with("\\source ")
            || trimmed.starts_with("\\. ")
    })
}

fn probable_function_definition(trimmed: &str) -> bool {
    if trimmed.starts_with('#') || trimmed.is_empty() {
        return false;
    }

    if let Some(rest) = trimmed.strip_prefix("function ") {
        return rest.contains('{');
    }

    trimmed.contains("() {") || trimmed.contains("(){")
}

fn source_mentions_any(source: &str, names: &[&str]) -> bool {
    names.iter().any(|name| source_mentions_name(source, name))
}

fn source_mentions_name(source: &str, name: &str) -> bool {
    source.contains(&format!("${name}"))
        || source.contains(&format!("${{{name}}}"))
        || source.contains(&format!("${{{name}:"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shuck_parser::parser::Parser;

    fn contract_for(path: &Path, source: &str) -> Option<FileContract> {
        let output = Parser::new(source).parse().unwrap();
        file_entry_contract(&output.file, source, Some(path), ShellDialect::Sh)
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
}
