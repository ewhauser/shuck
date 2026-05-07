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

use super::source_scan::shell_assignment_token;
use shuck_ast::{NormalizedCommand, Word, WrapperKind, normalize_command_words, static_word_text};

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
