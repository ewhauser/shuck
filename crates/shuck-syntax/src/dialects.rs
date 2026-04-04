/// Shell dialect requested by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Bash,
    Sh,
    Dash,
    Ksh,
    Mksh,
    Bats,
    Zsh,
}

impl Dialect {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Sh => "sh",
            Self::Dash => "dash",
            Self::Ksh => "ksh",
            Self::Mksh => "mksh",
            Self::Bats => "bats",
            Self::Zsh => "zsh",
        }
    }
}

impl std::fmt::Display for Dialect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parsing strategy requested by the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParseMode {
    Strict,
    StrictRecovered,
    Permissive,
    PermissiveRecovered,
}

impl ParseMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::StrictRecovered => "strict-recovered",
            Self::Permissive => "permissive",
            Self::PermissiveRecovered => "permissive-recovered",
        }
    }

    pub fn is_recovered(self) -> bool {
        matches!(self, Self::StrictRecovered | Self::PermissiveRecovered)
    }

    pub fn is_permissive(self) -> bool {
        matches!(self, Self::Permissive | Self::PermissiveRecovered)
    }
}

impl std::fmt::Display for ParseMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Initial parser options for the syntax wrapper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseOptions {
    pub dialect: Dialect,
    pub mode: ParseMode,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            dialect: Dialect::Bash,
            mode: ParseMode::Strict,
        }
    }
}

/// Concrete parser grammar selected by a parse view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grammar {
    Bash,
}

impl Grammar {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
        }
    }
}

impl std::fmt::Display for Grammar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether the selected parse view is native to the requested dialect or a tolerant fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ParseStrategy {
    Native,
    Permissive,
}

impl ParseStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Permissive => "permissive",
        }
    }
}

impl std::fmt::Display for ParseStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Resolved parse plan for a specific requested dialect and mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParseView {
    pub dialect: Dialect,
    pub mode: ParseMode,
    pub strategy: ParseStrategy,
    pub grammar: Grammar,
}

impl ParseView {
    pub fn is_recovered(self) -> bool {
        self.mode.is_recovered()
    }

    pub fn is_permissive(self) -> bool {
        self.strategy == ParseStrategy::Permissive
    }
}

/// Dialect-specific parse policy used to resolve native vs permissive views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DialectProfile {
    dialect: Dialect,
    native_grammar: Option<Grammar>,
    permissive_grammar: Option<Grammar>,
}

impl DialectProfile {
    pub fn for_dialect(dialect: Dialect) -> Self {
        let (native_grammar, permissive_grammar) = match dialect {
            Dialect::Bash => (Some(Grammar::Bash), Some(Grammar::Bash)),
            Dialect::Sh | Dialect::Dash | Dialect::Ksh | Dialect::Mksh => {
                (None, Some(Grammar::Bash))
            }
            Dialect::Bats | Dialect::Zsh => (None, None),
        };

        Self {
            dialect,
            native_grammar,
            permissive_grammar,
        }
    }

    pub fn dialect(self) -> Dialect {
        self.dialect
    }

    pub fn native_grammar(self) -> Option<Grammar> {
        self.native_grammar
    }

    pub fn permissive_grammar(self) -> Option<Grammar> {
        self.permissive_grammar
    }

    pub fn supports_mode(self, mode: ParseMode) -> bool {
        self.parse_view(mode).is_ok()
    }

    pub fn parse_view(self, mode: ParseMode) -> Result<ParseView, UnsupportedParseRequest> {
        let (strategy, grammar) = match mode {
            ParseMode::Strict | ParseMode::StrictRecovered => (
                ParseStrategy::Native,
                self.native_grammar.ok_or(UnsupportedParseRequest {
                    dialect: self.dialect,
                    mode,
                })?,
            ),
            ParseMode::Permissive | ParseMode::PermissiveRecovered => (
                ParseStrategy::Permissive,
                self.permissive_grammar.ok_or(UnsupportedParseRequest {
                    dialect: self.dialect,
                    mode,
                })?,
            ),
        };

        Ok(ParseView {
            dialect: self.dialect,
            mode,
            strategy,
            grammar,
        })
    }
}

impl From<Dialect> for DialectProfile {
    fn from(dialect: Dialect) -> Self {
        Self::for_dialect(dialect)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnsupportedParseRequest {
    pub dialect: Dialect,
    pub mode: ParseMode,
}

#[cfg(test)]
mod tests {
    use super::{Dialect, DialectProfile, Grammar, ParseMode, ParseStrategy, ParseView};

    #[test]
    fn bash_supports_native_and_permissive_views() {
        let profile = DialectProfile::for_dialect(Dialect::Bash);

        assert_eq!(profile.native_grammar(), Some(Grammar::Bash));
        assert_eq!(profile.permissive_grammar(), Some(Grammar::Bash));
        assert!(profile.supports_mode(ParseMode::Strict));
        assert!(profile.supports_mode(ParseMode::PermissiveRecovered));
        assert_eq!(
            profile.parse_view(ParseMode::Permissive).unwrap(),
            ParseView {
                dialect: Dialect::Bash,
                mode: ParseMode::Permissive,
                strategy: ParseStrategy::Permissive,
                grammar: Grammar::Bash,
            }
        );
    }

    #[test]
    fn posix_like_dialects_use_bash_as_permissive_fallback() {
        let profile = DialectProfile::for_dialect(Dialect::Dash);

        assert_eq!(profile.native_grammar(), None);
        assert_eq!(profile.permissive_grammar(), Some(Grammar::Bash));
        assert!(!profile.supports_mode(ParseMode::Strict));

        let view = profile.parse_view(ParseMode::PermissiveRecovered).unwrap();
        assert_eq!(view.dialect, Dialect::Dash);
        assert_eq!(view.mode, ParseMode::PermissiveRecovered);
        assert_eq!(view.strategy, ParseStrategy::Permissive);
        assert_eq!(view.grammar, Grammar::Bash);
        assert!(view.is_recovered());
    }

    #[test]
    fn zsh_has_no_parse_views_yet() {
        let profile = DialectProfile::for_dialect(Dialect::Zsh);

        assert!(!profile.supports_mode(ParseMode::Strict));
        assert!(!profile.supports_mode(ParseMode::Permissive));
    }
}
