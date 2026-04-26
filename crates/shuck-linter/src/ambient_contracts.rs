use std::collections::{BTreeSet, VecDeque};
use std::path::Path;

use shuck_ast::Name;
use shuck_semantic::{ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind};

use crate::{FileContext, ShellDialect};

struct AmbientContractProvider {
    matches: fn(source: &str, path: &Path, shell: ShellDialect, file_context: &FileContext) -> bool,
    build: fn(
        source: &str,
        path: &Path,
        shell: ShellDialect,
        file_context: &FileContext,
    ) -> FileContract,
}

pub(crate) fn file_entry_contract(
    source: &str,
    path: Option<&Path>,
    shell: ShellDialect,
    file_context: &FileContext,
) -> Option<FileContract> {
    let path = path?;
    let mut merged = FileContract::default();
    let mut matched = false;

    for provider in providers() {
        if (provider.matches)(source, path, shell, file_context) {
            matched = true;
            merge_contract(
                &mut merged,
                (provider.build)(source, path, shell, file_context),
            );
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
    source: &str,
    path: &Path,
    _shell: ShellDialect,
    file_context: &FileContext,
) -> bool {
    let lower = lower_path(path);
    sourced_runtime_path_shape(&lower) && sourced_runtime_source_shape(source, file_context, &lower)
}

fn build_sourced_runtime_contract(
    source: &str,
    path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> FileContract {
    let lower = lower_path(path);
    let mut names = BTreeSet::new();

    for name in runtime_names_for_source_path(source, &lower) {
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

fn sourced_runtime_source_shape(
    source: &str,
    _file_context: &FileContext,
    lower_path: &str,
) -> bool {
    has_probable_function_definition(source)
        || has_source_command(source)
        || source.contains("PROMPT_COMMAND")
        || source.contains("COMPREPLY")
        || source.contains("about-completion")
        || (lower_path.contains("termux-packages") && source.contains("TERMUX_"))
}

fn runtime_names_for_source_path(source: &str, lower: &str) -> &'static [&'static str] {
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

    if completion_runtime_shape(source, lower) {
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

fn completion_runtime_shape(source: &str, lower: &str) -> bool {
    completion_runtime_path_shape(lower) && completion_runtime_source_shape(source)
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

fn completion_runtime_source_shape(source: &str) -> bool {
    let mut pending_heredocs = VecDeque::new();

    for line in source.lines() {
        if skip_heredoc_body_line(line, &mut pending_heredocs) {
            continue;
        }

        if line_invokes_completion_initializer_command(line) {
            return true;
        }

        pending_heredocs.extend(heredoc_delimiters_in_code_line(line));
    }

    false
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingHeredocDelimiter {
    text: String,
    strip_tabs: bool,
}

fn skip_heredoc_body_line(
    line: &str,
    pending_heredocs: &mut VecDeque<PendingHeredocDelimiter>,
) -> bool {
    let Some(delimiter) = pending_heredocs.front() else {
        return false;
    };

    if heredoc_line_matches_delimiter(line, delimiter) {
        pending_heredocs.pop_front();
    }
    true
}

fn heredoc_line_matches_delimiter(line: &str, delimiter: &PendingHeredocDelimiter) -> bool {
    let candidate = if delimiter.strip_tabs {
        line.trim_start_matches('\t')
    } else {
        line
    };
    candidate == delimiter.text
}

fn heredoc_delimiters_in_code_line(line: &str) -> Vec<PendingHeredocDelimiter> {
    let mut delimiters = Vec::new();
    let mut index = 0;
    let mut escaped = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut previous = None;

    while index < line.len() {
        let ch = line[index..].chars().next().expect("index is in bounds");
        let next_index = index + ch.len_utf8();

        if in_single_quote {
            in_single_quote = ch != '\'';
            previous = Some(ch);
            index = next_index;
            continue;
        }

        if in_double_quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double_quote = false;
            }
            previous = Some(ch);
            index = next_index;
            continue;
        }

        if escaped {
            escaped = false;
            previous = Some(ch);
            index = next_index;
            continue;
        }

        if ch == '#' && shell_comment_can_start_after(previous) {
            break;
        }

        if ch == '<' && line[index..].starts_with("<<") && !line[index..].starts_with("<<<") {
            let strip_tabs = line[index..].starts_with("<<-");
            let delimiter_start = index + if strip_tabs { 3 } else { 2 };
            if let Some((delimiter, delimiter_end)) =
                parse_heredoc_delimiter(line, delimiter_start, strip_tabs)
            {
                delimiters.push(delimiter);
                previous = None;
                index = delimiter_end;
                continue;
            }
        }

        match ch {
            '\\' => escaped = true,
            '\'' => in_single_quote = true,
            '"' => in_double_quote = true,
            _ => {}
        }

        previous = Some(ch);
        index = next_index;
    }

    delimiters
}

fn parse_heredoc_delimiter(
    line: &str,
    mut index: usize,
    strip_tabs: bool,
) -> Option<(PendingHeredocDelimiter, usize)> {
    let mut skipped_spacing = false;
    while index < line.len() {
        let ch = line[index..].chars().next().expect("index is in bounds");
        if !matches!(ch, ' ' | '\t') {
            break;
        }
        skipped_spacing = true;
        index += ch.len_utf8();
    }

    if skipped_spacing && line[index..].starts_with('#') {
        return None;
    }

    let mut text = String::new();
    let mut escaped = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while index < line.len() {
        let ch = line[index..].chars().next().expect("index is in bounds");
        let next_index = index + ch.len_utf8();

        if escaped {
            text.push(ch);
            escaped = false;
            index = next_index;
            continue;
        }

        if in_single_quote {
            if ch == '\'' {
                in_single_quote = false;
            } else {
                text.push(ch);
            }
            index = next_index;
            continue;
        }

        if in_double_quote {
            if ch == '"' {
                in_double_quote = false;
            } else if ch == '\\' {
                escaped = true;
            } else {
                text.push(ch);
            }
            index = next_index;
            continue;
        }

        match ch {
            '\\' => escaped = true,
            '\'' => in_single_quote = true,
            '"' => in_double_quote = true,
            _ if heredoc_delimiter_terminator(ch) => break,
            _ => text.push(ch),
        }

        index = next_index;
    }

    (!text.is_empty()).then_some((PendingHeredocDelimiter { text, strip_tabs }, index))
}

fn heredoc_delimiter_terminator(ch: char) -> bool {
    ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '<' | '>' | '(' | ')' | '{' | '}')
}

fn line_invokes_completion_initializer_command(line: &str) -> bool {
    let mut command_position = true;
    let mut escaped = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut previous = None;
    let mut token_start = None;

    for (index, ch) in line.char_indices() {
        if in_single_quote {
            in_single_quote = ch != '\'';
            previous = Some(ch);
            continue;
        }

        if in_double_quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double_quote = false;
            }
            previous = Some(ch);
            continue;
        }

        if escaped {
            escaped = false;
            previous = Some(ch);
            continue;
        }

        if ch == '#' && shell_comment_can_start_after(previous) {
            if let Some(start) = token_start.take()
                && completion_initializer_token(
                    &line[start..index],
                    &line[index..],
                    &mut command_position,
                )
            {
                return true;
            }
            break;
        }

        if is_completion_runtime_token_char(ch) {
            token_start.get_or_insert(index);
            previous = Some(ch);
            continue;
        }

        if let Some(start) = token_start.take()
            && completion_initializer_token(
                &line[start..index],
                &line[index..],
                &mut command_position,
            )
        {
            return true;
        }

        match ch {
            '\\' => escaped = true,
            '\'' => in_single_quote = true,
            '"' => in_double_quote = true,
            ';' | '&' | '|' | '(' | '{' | '!' => command_position = true,
            _ => {}
        }
        previous = Some(ch);
    }

    token_start.is_some_and(|start| {
        completion_initializer_token(&line[start..], "", &mut command_position)
    })
}

fn completion_initializer_token(token: &str, following: &str, command_position: &mut bool) -> bool {
    if *command_position
        && is_completion_initializer_command(token)
        && !starts_function_definition_suffix(following)
    {
        return true;
    }

    if shell_assignment_token(token) {
        return false;
    }

    *command_position = shell_control_token_keeps_command_position(token);
    false
}

fn shell_comment_can_start_after(previous: Option<char>) -> bool {
    previous.is_none_or(|ch| ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '(' | '{'))
}

fn starts_function_definition_suffix(following: &str) -> bool {
    following.trim_start().starts_with("()")
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

fn shell_control_token_keeps_command_position(token: &str) -> bool {
    matches!(
        token,
        "if" | "then"
            | "do"
            | "else"
            | "elif"
            | "while"
            | "until"
            | "time"
            | "command"
            | "builtin"
            | "env"
    )
}

fn is_completion_runtime_token_char(ch: char) -> bool {
    ch == '_' || ch == '-' || ch == '=' || ch.is_ascii_alphanumeric()
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
    use crate::{FileContextTag, classify_file_context};

    fn contract_for(path: &Path, source: &str) -> Option<FileContract> {
        let context = classify_file_context(source, Some(path), ShellDialect::Sh);
        file_entry_contract(source, Some(path), ShellDialect::Sh, &context)
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

    #[test]
    fn broad_project_closure_tags_alone_do_not_inject_contracts() {
        let path = Path::new("/tmp/project/scripts/helper.sh");
        let source = "\
# shellcheck source=helper-lib.sh
. ./helper-lib.sh
printf '%s\\n' \"$pkgname\"
";
        let context = classify_file_context(source, Some(path), ShellDialect::Sh);
        assert!(context.has_tag(FileContextTag::ProjectClosure));

        assert!(file_entry_contract(source, Some(path), ShellDialect::Sh, &context).is_none());
    }
}
