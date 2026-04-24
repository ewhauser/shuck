use std::collections::{BTreeSet, VecDeque};
use std::path::Path;

use shuck_ast::Name;
use shuck_semantic::{ContractCertainty, FileContract, ProvidedBinding, ProvidedBindingKind};

use crate::{FileContext, FileContextTag, ShellDialect};

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
    &[
        AmbientContractProvider {
            matches: matches_void_packages_build_style_contract,
            build: build_void_packages_build_style_contract,
        },
        AmbientContractProvider {
            matches: matches_void_packages_pre_pkg_hook_contract,
            build: build_void_packages_pre_pkg_hook_contract,
        },
        AmbientContractProvider {
            matches: matches_void_packages_xbps_src_framework_contract,
            build: build_void_packages_xbps_src_framework_contract,
        },
        AmbientContractProvider {
            matches: matches_void_packages_pycompile_trigger_contract,
            build: build_void_packages_pycompile_trigger_contract,
        },
        AmbientContractProvider {
            matches: matches_sourced_runtime_contract,
            build: build_sourced_runtime_contract,
        },
    ]
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

fn matches_void_packages_build_style_contract(
    source: &str,
    path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> bool {
    let lower = lower_path(path);
    (path_matches_any(
        &lower,
        &[
            "void-packages/common/build-style/",
            "void-packages/common/environment/build-style/",
            "void-packages__common__build-style__",
            "void-packages__common__environment__build-style__",
        ],
    )) && has_probable_function_definition(source)
        && source_mentions_any(source, &["wrksrc", "XBPS_SRCPKGDIR"])
}

fn matches_void_packages_pre_pkg_hook_contract(
    source: &str,
    path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> bool {
    let lower = lower_path(path);
    path_matches_any(
        &lower,
        &[
            "void-packages/common/hooks/pre-pkg/",
            "void-packages__common__hooks__pre-pkg__",
        ],
    ) && (lower.contains("/99-pkglint") || lower.contains("__99-pkglint"))
        && lower.ends_with(".sh")
        && has_named_function_definition(source, "hook")
        && source.contains("PKGDESTDIR")
}

fn matches_void_packages_xbps_src_framework_contract(
    source: &str,
    path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> bool {
    let lower = lower_path(path);
    let libexec_path = path_matches_any(
        &lower,
        &[
            "void-packages/common/xbps-src/libexec/",
            "void-packages__common__xbps-src__libexec__",
        ],
    );
    let shutils_path = path_matches_any(
        &lower,
        &[
            "void-packages/common/xbps-src/shutils/",
            "void-packages__common__xbps-src__shutils__",
        ],
    );
    (libexec_path || shutils_path)
        && lower.ends_with(".sh")
        && xbps_src_framework_has_shell_shape(source, libexec_path)
        && source.matches("XBPS_").count() >= 3
}

fn matches_void_packages_pycompile_trigger_contract(
    source: &str,
    path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> bool {
    let lower = lower_path(path);
    (lower.ends_with("/void-packages/srcpkgs/xbps-triggers/files/pycompile")
        || lower.ends_with("__void-packages__srcpkgs__xbps-triggers__files__pycompile"))
        && (source.contains("ACTION=\"$1\"")
            || source.contains("TARGET=\"$2\"")
            || source.contains("case \"$ACTION\""))
}

fn build_void_packages_build_style_contract(
    _source: &str,
    _path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> FileContract {
    runtime_variable_contract(&[
        "build_style",
        "distfiles",
        "metapackage",
        "pkgname",
        "pkgver",
        "version",
        "pycompile_version",
        "XBPS_SRCPKGDIR",
        "XBPS_SRCDISTDIR",
        "XBPS_TARGET_WORDSIZE",
        "configure_args",
        "makejobs",
        "cross_binutils_configure_args",
        "cross_gcc_bootstrap_configure_args",
        "cross_gcc_configure_args",
        "cross_glibc_configure_args",
        "cross_musl_configure_args",
        "wrksrc",
    ])
}

fn build_void_packages_pre_pkg_hook_contract(
    _source: &str,
    _path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> FileContract {
    runtime_variable_contract(&[
        "PKGDESTDIR",
        "pkgname",
        "pkgver",
        "metapackage",
        "conf_files",
        "provides",
        "XBPS_COMMONDIR",
        "XBPS_STATEDIR",
        "XBPS_TARGET_MACHINE",
        "XBPS_QUERY_XCMD",
        "XBPS_UHELPER_CMD",
    ])
}

fn build_void_packages_xbps_src_framework_contract(
    _source: &str,
    _path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> FileContract {
    runtime_variable_contract(&[
        "XBPS_COMMONDIR",
        "XBPS_SRCPKGDIR",
        "XBPS_SRCDISTDIR",
        "XBPS_BUILDSTYLEDIR",
        "XBPS_LIBEXECDIR",
        "XBPS_STATEDIR",
        "XBPS_MACHINE",
        "XBPS_TARGET",
        "XBPS_TARGET_MACHINE",
        "XBPS_TARGET_PKG",
        "XBPS_CROSS_BUILD",
        "pkgname",
        "pkgver",
        "build_style",
        "sourcepkg",
        "subpackages",
        "wrksrc",
        "build_option_",
        "NOCOLORS",
        "XBPS_CFLAGS",
        "XBPS_CPPFLAGS",
        "XBPS_CXXFLAGS",
        "XBPS_FFLAGS",
        "XBPS_LDFLAGS",
    ])
}

fn build_void_packages_pycompile_trigger_contract(
    _source: &str,
    _path: &Path,
    _shell: ShellDialect,
    _file_context: &FileContext,
) -> FileContract {
    runtime_variable_contract(&["pycompile_dirs", "pycompile_module", "pycompile_version"])
}

fn runtime_variable_contract(names: &[&str]) -> FileContract {
    let mut contract = FileContract::default();
    for name in names {
        contract.add_provided_binding(ProvidedBinding::new(
            Name::from(*name),
            ProvidedBindingKind::Variable,
            ContractCertainty::Definite,
        ));
    }
    contract
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
        contract.add_provided_binding(ProvidedBinding::new_file_entry_initialized(
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
            "__completion__",
            "__completions-",
            "__completions__",
            "/themes/",
            ".theme.",
            "__themes__",
            "/plugins/",
            "/plugin/",
            "__plugins__",
            "/modules/",
            "__modules__",
            "/scriptmodules/",
            "__scriptmodules__",
            "/scripts/functions/",
            "__scripts__functions__",
            "/rvm/scripts/",
            "__rvm__scripts__",
            "/lgsm/modules/",
            "__lgsm__modules__",
            "/common/environment/setup/",
            "__common__environment__setup__",
            "/common/chroot-style/",
            "__common__chroot-style__",
            "/common/hooks/",
            "__common__hooks__",
            "termux-packages/packages/",
            "termux-packages__packages__",
        ],
    )
}

fn sourced_runtime_source_shape(
    source: &str,
    file_context: &FileContext,
    lower_path: &str,
) -> bool {
    file_context.has_tag(FileContextTag::HelperLibrary)
        || has_probable_function_definition(source)
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
    path_matches_any(
        lower,
        &[
            "/bash-it/themes/",
            "/bash-it/theme/",
            "__bash-it__themes__",
            "__bash-it__theme__",
        ],
    ) && (source.contains("PROMPT_COMMAND")
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
            "bash-completion__",
            "bash_completion__",
            "__bash-completion__",
            "__bash_completion__",
            "/bash-it/completion/",
            "/bash-it/completions/",
            "__bash-it__completion__",
            "__bash-it__completions__",
            "/bash-progcomp/",
            "__bash-progcomp__",
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

fn has_named_function_definition(source: &str, name: &str) -> bool {
    source
        .lines()
        .map(str::trim)
        .any(|trimmed| named_function_definition(trimmed, name))
}

fn has_source_command(source: &str) -> bool {
    source.lines().map(str::trim).any(|trimmed| {
        trimmed.starts_with("source ")
            || trimmed.starts_with(". ")
            || trimmed.starts_with("\\source ")
            || trimmed.starts_with("\\. ")
    })
}

fn xbps_src_framework_has_shell_shape(source: &str, libexec_path: bool) -> bool {
    has_probable_function_definition(source)
        || (libexec_path
            && source.contains("readonly XBPS_TARGET")
            && source.contains("setup_pkg \"$PKGNAME\""))
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

fn named_function_definition(trimmed: &str, name: &str) -> bool {
    if trimmed.starts_with('#') || trimmed.is_empty() {
        return false;
    }

    if let Some(rest) = trimmed.strip_prefix("function ") {
        let rest = rest.trim_start();
        return rest.starts_with(name) && rest.contains('{');
    }

    trimmed.starts_with(&format!("{name}()"))
        || trimmed.starts_with(&format!("{name} ()"))
        || trimmed.contains(&format!("{name}() {{"))
        || trimmed.contains(&format!("{name}(){{"))
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

    fn has_binding(contract: &FileContract, name: &str) -> bool {
        contract
            .provided_bindings
            .iter()
            .any(|binding| binding.name == name)
    }

    fn has_initialized_binding(contract: &FileContract, name: &str) -> bool {
        contract.provided_bindings.iter().any(|binding| {
            binding.name == name
                && binding.file_entry_initialization
                    == shuck_semantic::FileEntryBindingInitialization::Initialized
        })
    }

    #[test]
    fn void_packages_build_style_paths_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/common/build-style/void-cross.sh");
        let source = "\
helper() { cd \"${wrksrc}\"; }
printf '%s\\n' \"$pkgname\" \"$pkgver\" \"$XBPS_SRCPKGDIR\" \"$configure_args\"
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_binding(&contract, "pkgname"));
        assert!(has_binding(&contract, "pkgver"));
        assert!(has_binding(&contract, "wrksrc"));
        assert!(has_binding(&contract, "XBPS_SRCPKGDIR"));
        assert!(has_binding(&contract, "configure_args"));
        assert!(!has_initialized_binding(&contract, "wrksrc"));
        assert!(!contract.externally_consumed_bindings);
    }

    #[test]
    fn void_packages_pre_pkg_hook_paths_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/common/hooks/pre-pkg/99-pkglint.sh");
        let source = "\
hook() { printf '%s\\n' \"$PKGDESTDIR\"; }
printf '%s\\n' \"$pkgname\" \"$pkgver\" \"$XBPS_COMMONDIR\"
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_binding(&contract, "PKGDESTDIR"));
        assert!(has_binding(&contract, "pkgname"));
        assert!(has_binding(&contract, "pkgver"));
        assert!(has_binding(&contract, "XBPS_COMMONDIR"));
        assert!(!has_initialized_binding(&contract, "pkgname"));
        assert!(!contract.externally_consumed_bindings);
    }

    #[test]
    fn void_packages_xbps_src_framework_paths_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/common/xbps-src/shutils/common.sh");
        let source = "\
helper() { printf '%s\\n' \"$XBPS_COMMONDIR\"; }
printf '%s\\n' \"$XBPS_SRCPKGDIR\" \"$XBPS_STATEDIR\" \"$pkgname\" \"$build_style\"
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_binding(&contract, "XBPS_COMMONDIR"));
        assert!(has_binding(&contract, "XBPS_SRCPKGDIR"));
        assert!(has_binding(&contract, "XBPS_STATEDIR"));
        assert!(has_binding(&contract, "build_style"));
        assert!(!has_initialized_binding(&contract, "build_style"));
        assert!(!contract.externally_consumed_bindings);
    }

    #[test]
    fn void_packages_xbps_src_libexec_drivers_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/common/xbps-src/libexec/build.sh");
        let source = "\
readonly XBPS_TARGET=\"$1\"
setup_pkg \"$PKGNAME\"
for subpkg in ${subpackages} ${sourcepkg}; do
  printf '%s\\n' \"$XBPS_LIBEXECDIR\" \"$XBPS_CROSS_BUILD\"
done
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_binding(&contract, "sourcepkg"));
        assert!(has_binding(&contract, "subpackages"));
        assert!(has_binding(&contract, "XBPS_LIBEXECDIR"));
        assert!(has_binding(&contract, "XBPS_CROSS_BUILD"));
        assert!(!has_initialized_binding(&contract, "sourcepkg"));
        assert!(!contract.externally_consumed_bindings);
    }

    #[test]
    fn void_packages_pycompile_trigger_paths_get_an_explicit_contract() {
        let path = Path::new("/tmp/void-packages/srcpkgs/xbps-triggers/files/pycompile");
        let source = "\
ACTION=\"$1\"
TARGET=\"$2\"
case \"$ACTION\" in
run) printf '%s\\n' \"$pycompile_dirs\" \"$pycompile_module\" \"$pycompile_version\" ;;
esac
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_binding(&contract, "pycompile_dirs"));
        assert!(has_binding(&contract, "pycompile_module"));
        assert!(has_binding(&contract, "pycompile_version"));
        assert!(!has_initialized_binding(&contract, "pycompile_version"));
        assert!(!contract.externally_consumed_bindings);
    }

    #[test]
    fn bash_it_theme_paths_get_palette_initialized_contracts() {
        let path = Path::new("/tmp/Bash-it/themes/example/example.theme.bash");
        let source = "\
prompt_command() {
  PS1=\"${green?} ${green} ${reset_color?}\"
}
PROMPT_COMMAND=prompt_command
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_initialized_binding(&contract, "green"));
        assert!(has_initialized_binding(&contract, "reset_color"));
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
    fn bash_completion_paths_with_initializer_get_completion_contracts() {
        let path = Path::new("/tmp/bash-completion/completions/example.bash");
        let source = "\
_example() {
  _init_completion || return
  printf '%s\\n' \"$cur\" \"$cword\" \"$comp_args\"
}
";

        let contract = contract_for(path, source).unwrap();

        assert!(has_initialized_binding(&contract, "cur"));
        assert!(has_initialized_binding(&contract, "cword"));
        assert!(has_initialized_binding(&contract, "comp_args"));
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

    #[test]
    fn void_packages_paths_without_required_source_anchors_do_not_inject_contracts() {
        let xbps_src_path = Path::new("/tmp/void-packages/common/xbps-src/shutils/common.sh");
        let xbps_src_source = "printf '%s\\n' \"$XBPS_COMMONDIR\"\n";
        assert!(contract_for(xbps_src_path, xbps_src_source).is_none());

        let pycompile_path = Path::new("/tmp/void-packages/srcpkgs/xbps-triggers/files/pycompile");
        let pycompile_source = "printf '%s\\n' \"$pycompile_version\"\n";
        assert!(contract_for(pycompile_path, pycompile_source).is_none());
    }

    #[test]
    fn flattened_large_corpus_void_packages_paths_also_get_contracts() {
        let build_style_path =
            Path::new("/tmp/scripts/void-linux__void-packages__common__build-style__void-cross.sh");
        let build_style_source = "\
helper() { :; }
printf '%s\\n' \"$XBPS_SRCPKGDIR\" \"$configure_args\" \"$wrksrc\"
";
        let build_style_contract = contract_for(build_style_path, build_style_source).unwrap();
        assert!(has_binding(&build_style_contract, "wrksrc"));
        assert!(has_binding(&build_style_contract, "configure_args"));

        let pycompile_path = Path::new(
            "/tmp/scripts/void-linux__void-packages__srcpkgs__xbps-triggers__files__pycompile",
        );
        let pycompile_source = "\
ACTION=\"$1\"
case \"$ACTION\" in
run) printf '%s\\n' \"$pycompile_version\" ;;
esac
";
        let pycompile_contract = contract_for(pycompile_path, pycompile_source).unwrap();
        assert!(has_binding(&pycompile_contract, "pycompile_version"));
    }
}
