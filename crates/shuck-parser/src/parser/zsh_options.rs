/// Tri-state option value used when modeling zsh option state.
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
    pub const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::On, Self::On) => Self::On,
            (Self::Off, Self::Off) => Self::Off,
            _ => Self::Unknown,
        }
    }
}

/// Target emulation mode for zsh's `emulate` behavior.
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

/// Snapshot of zsh option state used by the parser and lexer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ZshOptionState {
    /// State of the `sh_word_split` option.
    pub sh_word_split: OptionValue,
    /// State of the `glob_subst` option.
    pub glob_subst: OptionValue,
    /// State of the `rc_expand_param` option.
    pub rc_expand_param: OptionValue,
    /// State of the `glob` option.
    pub glob: OptionValue,
    /// State of the `nomatch` option.
    pub nomatch: OptionValue,
    /// State of the `null_glob` option.
    pub null_glob: OptionValue,
    /// State of the `csh_null_glob` option.
    pub csh_null_glob: OptionValue,
    /// State of the `extended_glob` option.
    pub extended_glob: OptionValue,
    /// State of the `ksh_glob` option.
    pub ksh_glob: OptionValue,
    /// State of the `sh_glob` option.
    pub sh_glob: OptionValue,
    /// State of the `bare_glob_qual` option.
    pub bare_glob_qual: OptionValue,
    /// State of the `glob_dots` option.
    pub glob_dots: OptionValue,
    /// State of the `equals` option.
    pub equals: OptionValue,
    /// State of the `magic_equal_subst` option.
    pub magic_equal_subst: OptionValue,
    /// State of the `sh_file_expansion` option.
    pub sh_file_expansion: OptionValue,
    /// State of the `glob_assign` option.
    pub glob_assign: OptionValue,
    /// State of the `ignore_braces` option.
    pub ignore_braces: OptionValue,
    /// State of the `ignore_close_braces` option.
    pub ignore_close_braces: OptionValue,
    /// State of the `brace_ccl` option.
    pub brace_ccl: OptionValue,
    /// State of the `ksh_arrays` option.
    pub ksh_arrays: OptionValue,
    /// State of the `ksh_zero_subscript` option.
    pub ksh_zero_subscript: OptionValue,
    /// State of the `short_loops` option.
    pub short_loops: OptionValue,
    /// State of the `short_repeat` option.
    pub short_repeat: OptionValue,
    /// State of the `rc_quotes` option.
    pub rc_quotes: OptionValue,
    /// State of the `interactive_comments` option.
    pub interactive_comments: OptionValue,
    /// State of the `c_bases` option.
    pub c_bases: OptionValue,
    /// State of the `octal_zeroes` option.
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

    /// Option state implied by `emulate <mode>`.
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

    /// Apply a zsh `setopt`-style option name.
    ///
    /// Returns `true` when the option name was recognized.
    pub fn apply_setopt(&mut self, name: &str) -> bool {
        self.apply_named_option(name, true)
    }

    /// Apply a zsh `unsetopt`-style option name.
    ///
    /// Returns `true` when the option name was recognized.
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
