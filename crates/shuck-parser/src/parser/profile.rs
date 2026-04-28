use super::ZshOptionState;

/// Supported shell dialects for parser syntax decisions.
///
/// Dialects select which grammar extensions the parser accepts. They do not
/// try to model every runtime behavior of a shell; use [`ShellProfile`] when
/// zsh option state also matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShellDialect {
    /// POSIX-style parsing used for `sh`, `dash`, and generic portable shell input.
    Posix,
    /// mksh-specific parsing.
    Mksh,
    /// Bash parsing.
    #[default]
    Bash,
    /// zsh parsing.
    Zsh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DialectFeatures {
    pub(super) double_bracket: bool,
    pub(super) arithmetic_command: bool,
    pub(super) arithmetic_for: bool,
    pub(super) function_keyword: bool,
    pub(super) select_loop: bool,
    pub(super) coproc_keyword: bool,
    pub(super) zsh_repeat_loop: bool,
    pub(super) zsh_foreach_loop: bool,
    pub(super) zsh_parameter_modifiers: bool,
    pub(super) zsh_brace_if: bool,
    pub(super) zsh_always: bool,
    pub(super) zsh_background_operators: bool,
    pub(super) zsh_glob_qualifiers: bool,
}

impl ShellDialect {
    /// Infer a parser dialect from a command name, shebang interpreter name,
    /// or user-facing shell selector.
    ///
    /// Unknown names fall back to [`ShellDialect::Bash`], matching Shuck's
    /// default parsing mode.
    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "sh" | "dash" | "ksh" | "posix" => Self::Posix,
            "mksh" => Self::Mksh,
            "zsh" => Self::Zsh,
            _ => Self::Bash,
        }
    }

    pub(super) const fn features(self) -> DialectFeatures {
        match self {
            Self::Posix => DialectFeatures {
                double_bracket: false,
                arithmetic_command: false,
                arithmetic_for: false,
                function_keyword: true,
                select_loop: false,
                coproc_keyword: false,
                zsh_repeat_loop: false,
                zsh_foreach_loop: false,
                zsh_parameter_modifiers: false,
                zsh_brace_if: false,
                zsh_always: false,
                zsh_background_operators: false,
                zsh_glob_qualifiers: false,
            },
            Self::Mksh => DialectFeatures {
                double_bracket: true,
                arithmetic_command: true,
                arithmetic_for: false,
                function_keyword: true,
                select_loop: true,
                coproc_keyword: false,
                zsh_repeat_loop: false,
                zsh_foreach_loop: false,
                zsh_parameter_modifiers: false,
                zsh_brace_if: false,
                zsh_always: false,
                zsh_background_operators: false,
                zsh_glob_qualifiers: false,
            },
            Self::Bash => DialectFeatures {
                double_bracket: true,
                arithmetic_command: true,
                arithmetic_for: true,
                function_keyword: true,
                select_loop: true,
                coproc_keyword: true,
                zsh_repeat_loop: false,
                zsh_foreach_loop: false,
                zsh_parameter_modifiers: false,
                zsh_brace_if: false,
                zsh_always: false,
                zsh_background_operators: false,
                zsh_glob_qualifiers: false,
            },
            Self::Zsh => DialectFeatures {
                double_bracket: true,
                arithmetic_command: true,
                arithmetic_for: true,
                function_keyword: true,
                select_loop: true,
                coproc_keyword: true,
                zsh_repeat_loop: true,
                zsh_foreach_loop: true,
                zsh_parameter_modifiers: true,
                zsh_brace_if: true,
                zsh_always: true,
                zsh_background_operators: true,
                zsh_glob_qualifiers: true,
            },
        }
    }
}

/// Complete shell parsing profile.
///
/// A profile combines the broad syntax dialect with any option state that
/// changes tokenization or grammar. Today only zsh carries parser-visible
/// options; non-zsh profiles ignore the [`ShellProfile::options`] field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ShellProfile {
    /// Shell dialect to parse.
    pub dialect: ShellDialect,
    /// Optional zsh option state, used only when [`ShellProfile::dialect`] is
    /// [`ShellDialect::Zsh`].
    pub options: Option<ZshOptionState>,
}

impl ShellProfile {
    /// Build the parser's native profile for `dialect`.
    ///
    /// Native zsh profiles include [`ZshOptionState::zsh_default`]. Other
    /// dialects carry no option state.
    pub fn native(dialect: ShellDialect) -> Self {
        Self {
            dialect,
            options: (dialect == ShellDialect::Zsh).then(ZshOptionState::zsh_default),
        }
    }

    /// Build a profile with explicit zsh option state.
    ///
    /// The provided options are retained only for [`ShellDialect::Zsh`]. For
    /// other dialects, this returns a profile with `options: None` because
    /// their parser behavior is not currently option-sensitive.
    pub fn with_zsh_options(dialect: ShellDialect, options: ZshOptionState) -> Self {
        Self {
            dialect,
            options: (dialect == ShellDialect::Zsh).then_some(options),
        }
    }

    /// Borrow the zsh option state, if this profile carries one.
    ///
    /// Callers should treat `None` as "no parser-visible zsh option state",
    /// not as "all options are unknown".
    pub fn zsh_options(&self) -> Option<&ZshOptionState> {
        self.options.as_ref()
    }
}
