/// Tri-state value for a parser-visible zsh option.
///
/// `Unknown` is used when the parser cannot prove a single value, for example
/// after merging control-flow paths that set an option differently. Consumers
/// should only branch on [`OptionValue::On`] or [`OptionValue::Off`] when the
/// corresponding `is_definitely_*` helper returns `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum OptionValue {
    /// The option is enabled.
    On,
    /// The option is disabled.
    Off,
    /// The option value is unknown or differs across merged states.
    #[default]
    Unknown,
}

impl OptionValue {
    /// Returns `true` when the option is known to be enabled.
    pub const fn is_definitely_on(self) -> bool {
        matches!(self, Self::On)
    }

    /// Returns `true` when the option is known to be disabled.
    pub const fn is_definitely_off(self) -> bool {
        matches!(self, Self::Off)
    }

    /// Merge two option values, preserving certainty only when they agree.
    ///
    /// This is intended for conservative flow joins: `On + On` remains `On`,
    /// `Off + Off` remains `Off`, and every mixed or unknown combination
    /// becomes [`OptionValue::Unknown`].
    pub const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::On, Self::On) => Self::On,
            (Self::Off, Self::Off) => Self::Off,
            _ => Self::Unknown,
        }
    }
}

/// Target emulation mode for zsh's `emulate` behavior.
///
/// The parser uses this to derive the option snapshot implied by commands such
/// as `emulate sh` or `emulate ksh`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZshEmulationMode {
    /// Native zsh behavior.
    Zsh,
    /// `sh` compatibility mode.
    Sh,
    /// `ksh` compatibility mode.
    Ksh,
    /// `csh` compatibility mode.
    Csh,
}

/// Snapshot of parser-visible zsh option state.
///
/// The fields here intentionally cover options that can change syntax,
/// tokenization, or word interpretation. They are not a full zsh runtime option
/// table. Use [`ZshOptionState::zsh_default`] for native zsh parsing, then
/// apply `setopt`, `unsetopt`, or `emulate` effects when a caller has already
/// discovered them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ZshOptionState {
    /// Whether unquoted parameter expansion is treated as eligible for
    /// shell-style word splitting.
    pub sh_word_split: OptionValue,
    /// Whether parameter expansion results are treated as glob patterns.
    pub glob_subst: OptionValue,
    /// Whether array parameters can participate in brace-like expansion.
    pub rc_expand_param: OptionValue,
    /// Whether ordinary filename generation is enabled.
    pub glob: OptionValue,
    /// Whether unmatched filename-generation patterns are treated as errors.
    pub nomatch: OptionValue,
    /// Whether unmatched filename-generation patterns can expand to nothing.
    pub null_glob: OptionValue,
    /// Whether csh-style null glob handling is enabled.
    pub csh_null_glob: OptionValue,
    /// Whether zsh extended glob operators are enabled.
    pub extended_glob: OptionValue,
    /// Whether ksh-style glob operators are enabled.
    pub ksh_glob: OptionValue,
    /// Whether sh-compatible glob parsing is enabled.
    pub sh_glob: OptionValue,
    /// Whether unparenthesized zsh glob qualifiers are enabled.
    pub bare_glob_qual: OptionValue,
    /// Whether glob patterns match dotfiles without an explicit dot.
    pub glob_dots: OptionValue,
    /// Whether leading `=` words are eligible for command-path expansion.
    pub equals: OptionValue,
    /// Whether assignment-like words can apply `=` expansion after the first
    /// equals sign.
    pub magic_equal_subst: OptionValue,
    /// Whether file expansion follows sh-compatible ordering.
    pub sh_file_expansion: OptionValue,
    /// Whether assignment values can be parsed as glob assignments.
    pub glob_assign: OptionValue,
    /// Whether brace characters should be treated literally instead of as zsh
    /// brace syntax.
    pub ignore_braces: OptionValue,
    /// Whether unmatched closing braces should be treated literally.
    pub ignore_close_braces: OptionValue,
    /// Whether character-class brace expansion syntax is enabled.
    pub brace_ccl: OptionValue,
    /// Whether array indexing follows ksh-style zero-based behavior.
    pub ksh_arrays: OptionValue,
    /// Whether subscript zero is accepted with ksh-style array semantics.
    pub ksh_zero_subscript: OptionValue,
    /// Whether zsh short loop forms are accepted.
    pub short_loops: OptionValue,
    /// Whether zsh short `repeat` forms are accepted.
    pub short_repeat: OptionValue,
    /// Whether doubled single quotes are decoded inside single-quoted strings.
    pub rc_quotes: OptionValue,
    /// Whether `#` starts comments in interactive-style zsh parsing contexts.
    pub interactive_comments: OptionValue,
    /// Whether C-style numeric base prefixes are accepted in arithmetic text.
    pub c_bases: OptionValue,
    /// Whether leading zeroes are interpreted as octal arithmetic literals.
    pub octal_zeroes: OptionValue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ZshOptionField {
    ShWordSplit,
    GlobSubst,
    RcExpandParam,
    Glob,
    Nomatch,
    NullGlob,
    CshNullGlob,
    ExtendedGlob,
    KshGlob,
    ShGlob,
    BareGlobQual,
    GlobDots,
    Equals,
    MagicEqualSubst,
    ShFileExpansion,
    GlobAssign,
    IgnoreBraces,
    IgnoreCloseBraces,
    BraceCcl,
    KshArrays,
    KshZeroSubscript,
    ShortLoops,
    ShortRepeat,
    RcQuotes,
    InteractiveComments,
    CBases,
    OctalZeroes,
}

impl ZshOptionState {
    /// Default zsh option state used for native zsh parsing.
    ///
    /// This is the parser's baseline before source-level commands such as
    /// `emulate`, `setopt`, and `unsetopt` are considered.
    pub const fn zsh_default() -> Self {
        Self {
            sh_word_split: OptionValue::Off,
            glob_subst: OptionValue::Off,
            rc_expand_param: OptionValue::Off,
            glob: OptionValue::On,
            nomatch: OptionValue::On,
            null_glob: OptionValue::Off,
            csh_null_glob: OptionValue::Off,
            extended_glob: OptionValue::Off,
            ksh_glob: OptionValue::Off,
            sh_glob: OptionValue::Off,
            bare_glob_qual: OptionValue::On,
            glob_dots: OptionValue::Off,
            equals: OptionValue::On,
            magic_equal_subst: OptionValue::Off,
            sh_file_expansion: OptionValue::Off,
            glob_assign: OptionValue::Off,
            ignore_braces: OptionValue::Off,
            ignore_close_braces: OptionValue::Off,
            brace_ccl: OptionValue::Off,
            ksh_arrays: OptionValue::Off,
            ksh_zero_subscript: OptionValue::Off,
            short_loops: OptionValue::On,
            short_repeat: OptionValue::On,
            rc_quotes: OptionValue::Off,
            interactive_comments: OptionValue::On,
            c_bases: OptionValue::Off,
            octal_zeroes: OptionValue::Off,
        }
    }

    /// Return the option state implied by `emulate <mode>`.
    ///
    /// This models the subset of emulation effects that the parser currently
    /// needs. Callers can further refine the returned state with
    /// [`ZshOptionState::apply_setopt`] and [`ZshOptionState::apply_unsetopt`].
    pub fn for_emulate(mode: ZshEmulationMode) -> Self {
        let mut state = Self::zsh_default();
        match mode {
            ZshEmulationMode::Zsh => {}
            ZshEmulationMode::Sh => {
                state.sh_word_split = OptionValue::On;
                state.glob_subst = OptionValue::On;
                state.sh_glob = OptionValue::On;
                state.sh_file_expansion = OptionValue::On;
                state.bare_glob_qual = OptionValue::Off;
                state.ksh_arrays = OptionValue::Off;
            }
            ZshEmulationMode::Ksh => {
                state.sh_word_split = OptionValue::On;
                state.glob_subst = OptionValue::On;
                state.ksh_glob = OptionValue::On;
                state.ksh_arrays = OptionValue::On;
                state.sh_glob = OptionValue::On;
                state.bare_glob_qual = OptionValue::Off;
            }
            ZshEmulationMode::Csh => {
                state.csh_null_glob = OptionValue::On;
                state.sh_word_split = OptionValue::Off;
                state.glob_subst = OptionValue::Off;
            }
        }
        state
    }

    /// Apply a zsh `setopt`-style option name to this snapshot.
    ///
    /// Names are matched with zsh-style aliases, underscores, and `no_`
    /// prefixes where supported by this parser. Returns `true` when the option
    /// name was recognized and this snapshot was updated.
    pub fn apply_setopt(&mut self, name: &str) -> bool {
        self.apply_named_option(name, true)
    }

    /// Apply a zsh `unsetopt`-style option name to this snapshot.
    ///
    /// Names are matched with zsh-style aliases, underscores, and `no_`
    /// prefixes where supported by this parser. Returns `true` when the option
    /// name was recognized and this snapshot was updated.
    pub fn apply_unsetopt(&mut self, name: &str) -> bool {
        self.apply_named_option(name, false)
    }

    fn set_field(&mut self, field: ZshOptionField, value: OptionValue) {
        match field {
            ZshOptionField::ShWordSplit => self.sh_word_split = value,
            ZshOptionField::GlobSubst => self.glob_subst = value,
            ZshOptionField::RcExpandParam => self.rc_expand_param = value,
            ZshOptionField::Glob => self.glob = value,
            ZshOptionField::Nomatch => self.nomatch = value,
            ZshOptionField::NullGlob => self.null_glob = value,
            ZshOptionField::CshNullGlob => self.csh_null_glob = value,
            ZshOptionField::ExtendedGlob => self.extended_glob = value,
            ZshOptionField::KshGlob => self.ksh_glob = value,
            ZshOptionField::ShGlob => self.sh_glob = value,
            ZshOptionField::BareGlobQual => self.bare_glob_qual = value,
            ZshOptionField::GlobDots => self.glob_dots = value,
            ZshOptionField::Equals => self.equals = value,
            ZshOptionField::MagicEqualSubst => self.magic_equal_subst = value,
            ZshOptionField::ShFileExpansion => self.sh_file_expansion = value,
            ZshOptionField::GlobAssign => self.glob_assign = value,
            ZshOptionField::IgnoreBraces => self.ignore_braces = value,
            ZshOptionField::IgnoreCloseBraces => self.ignore_close_braces = value,
            ZshOptionField::BraceCcl => self.brace_ccl = value,
            ZshOptionField::KshArrays => self.ksh_arrays = value,
            ZshOptionField::KshZeroSubscript => self.ksh_zero_subscript = value,
            ZshOptionField::ShortLoops => self.short_loops = value,
            ZshOptionField::ShortRepeat => self.short_repeat = value,
            ZshOptionField::RcQuotes => self.rc_quotes = value,
            ZshOptionField::InteractiveComments => self.interactive_comments = value,
            ZshOptionField::CBases => self.c_bases = value,
            ZshOptionField::OctalZeroes => self.octal_zeroes = value,
        }
    }

    fn field(&self, field: ZshOptionField) -> OptionValue {
        match field {
            ZshOptionField::ShWordSplit => self.sh_word_split,
            ZshOptionField::GlobSubst => self.glob_subst,
            ZshOptionField::RcExpandParam => self.rc_expand_param,
            ZshOptionField::Glob => self.glob,
            ZshOptionField::Nomatch => self.nomatch,
            ZshOptionField::NullGlob => self.null_glob,
            ZshOptionField::CshNullGlob => self.csh_null_glob,
            ZshOptionField::ExtendedGlob => self.extended_glob,
            ZshOptionField::KshGlob => self.ksh_glob,
            ZshOptionField::ShGlob => self.sh_glob,
            ZshOptionField::BareGlobQual => self.bare_glob_qual,
            ZshOptionField::GlobDots => self.glob_dots,
            ZshOptionField::Equals => self.equals,
            ZshOptionField::MagicEqualSubst => self.magic_equal_subst,
            ZshOptionField::ShFileExpansion => self.sh_file_expansion,
            ZshOptionField::GlobAssign => self.glob_assign,
            ZshOptionField::IgnoreBraces => self.ignore_braces,
            ZshOptionField::IgnoreCloseBraces => self.ignore_close_braces,
            ZshOptionField::BraceCcl => self.brace_ccl,
            ZshOptionField::KshArrays => self.ksh_arrays,
            ZshOptionField::KshZeroSubscript => self.ksh_zero_subscript,
            ZshOptionField::ShortLoops => self.short_loops,
            ZshOptionField::ShortRepeat => self.short_repeat,
            ZshOptionField::RcQuotes => self.rc_quotes,
            ZshOptionField::InteractiveComments => self.interactive_comments,
            ZshOptionField::CBases => self.c_bases,
            ZshOptionField::OctalZeroes => self.octal_zeroes,
        }
    }

    /// Merge two option snapshots field by field.
    ///
    /// Each field preserves a definite value only when both inputs agree. This
    /// is useful for conservative joins across control-flow paths.
    pub fn merge(&self, other: &Self) -> Self {
        let mut merged = Self::zsh_default();
        for field in ZshOptionField::ALL {
            merged.set_field(field, self.field(field).merge(other.field(field)));
        }
        merged
    }

    fn apply_named_option(&mut self, name: &str, enable: bool) -> bool {
        let Some((field, value)) = parse_zsh_option_assignment(name, enable) else {
            return false;
        };
        self.set_field(
            field,
            if value {
                OptionValue::On
            } else {
                OptionValue::Off
            },
        );
        true
    }
}

impl ZshOptionField {
    const ALL: [Self; 27] = [
        Self::ShWordSplit,
        Self::GlobSubst,
        Self::RcExpandParam,
        Self::Glob,
        Self::Nomatch,
        Self::NullGlob,
        Self::CshNullGlob,
        Self::ExtendedGlob,
        Self::KshGlob,
        Self::ShGlob,
        Self::BareGlobQual,
        Self::GlobDots,
        Self::Equals,
        Self::MagicEqualSubst,
        Self::ShFileExpansion,
        Self::GlobAssign,
        Self::IgnoreBraces,
        Self::IgnoreCloseBraces,
        Self::BraceCcl,
        Self::KshArrays,
        Self::KshZeroSubscript,
        Self::ShortLoops,
        Self::ShortRepeat,
        Self::RcQuotes,
        Self::InteractiveComments,
        Self::CBases,
        Self::OctalZeroes,
    ];
}

fn parse_zsh_option_assignment(name: &str, enable: bool) -> Option<(ZshOptionField, bool)> {
    let mut normalized = String::with_capacity(name.len());
    for ch in name.chars() {
        if matches!(ch, '_' | '-') {
            continue;
        }
        normalized.push(ch.to_ascii_lowercase());
    }

    let (normalized, invert) = if let Some(rest) = normalized.strip_prefix("no") {
        (rest, true)
    } else {
        (normalized.as_str(), false)
    };

    let field = match normalized {
        "shwordsplit" => ZshOptionField::ShWordSplit,
        "globsubst" => ZshOptionField::GlobSubst,
        "rcexpandparam" => ZshOptionField::RcExpandParam,
        "glob" | "noglob" => ZshOptionField::Glob,
        "nomatch" => ZshOptionField::Nomatch,
        "nullglob" => ZshOptionField::NullGlob,
        "cshnullglob" => ZshOptionField::CshNullGlob,
        "extendedglob" => ZshOptionField::ExtendedGlob,
        "kshglob" => ZshOptionField::KshGlob,
        "shglob" => ZshOptionField::ShGlob,
        "bareglobqual" => ZshOptionField::BareGlobQual,
        "globdots" => ZshOptionField::GlobDots,
        "equals" => ZshOptionField::Equals,
        "magicequalsubst" => ZshOptionField::MagicEqualSubst,
        "shfileexpansion" => ZshOptionField::ShFileExpansion,
        "globassign" => ZshOptionField::GlobAssign,
        "ignorebraces" => ZshOptionField::IgnoreBraces,
        "ignoreclosebraces" => ZshOptionField::IgnoreCloseBraces,
        "braceccl" => ZshOptionField::BraceCcl,
        "ksharrays" => ZshOptionField::KshArrays,
        "kshzerosubscript" => ZshOptionField::KshZeroSubscript,
        "shortloops" => ZshOptionField::ShortLoops,
        "shortrepeat" => ZshOptionField::ShortRepeat,
        "rcquotes" => ZshOptionField::RcQuotes,
        "interactivecomments" => ZshOptionField::InteractiveComments,
        "cbases" => ZshOptionField::CBases,
        "octalzeroes" => ZshOptionField::OctalZeroes,
        _ => return None,
    };

    Some((field, if invert { !enable } else { enable }))
}
