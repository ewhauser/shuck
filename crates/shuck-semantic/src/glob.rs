use crate::{OptionValue, ShellBehaviorAt, ShellDialect};

/// Whether unquoted expansion results are subject to field splitting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldSplittingBehavior {
    /// Field splitting does not apply.
    Never,
    /// Field splitting applies only when the expansion is unquoted.
    UnquotedOnly,
    /// Runtime option state may select either behavior.
    Ambiguous,
}

impl FieldSplittingBehavior {
    /// Returns whether an unquoted expansion result may be split into fields.
    pub fn unquoted_results_can_split(self) -> bool {
        matches!(self, Self::UnquotedOnly | Self::Ambiguous)
    }
}

/// Whether pathname expansion applies to literal globs and substitution results.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathnameExpansionBehavior {
    /// Pathname expansion does not apply.
    Disabled,
    /// Literal glob characters are active, but substitution results are not globbed.
    LiteralGlobsOnly,
    /// Runtime option state may enable literal glob expansion, but substitution results stay plain.
    LiteralGlobsOnlyOrDisabled,
    /// Unquoted substitution results can also trigger pathname expansion.
    SubstitutionResultsWhenUnquoted,
    /// Runtime option state may select either behavior family.
    Ambiguous,
}

impl PathnameExpansionBehavior {
    /// Returns whether literal glob characters may trigger pathname expansion.
    pub fn literal_globs_can_expand(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    /// Returns whether unquoted substitution results may be interpreted as globs.
    pub fn unquoted_substitution_results_can_glob(self) -> bool {
        matches!(
            self,
            Self::SubstitutionResultsWhenUnquoted | Self::Ambiguous
        )
    }
}

/// How unmatched glob patterns behave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobFailureBehavior {
    /// An unmatched glob produces an error.
    ErrorOnNoMatch,
    /// An unmatched glob stays literal in argv.
    KeepLiteralOnNoMatch,
    /// An unmatched glob is removed from argv.
    DropUnmatchedPattern,
    /// Unmatched globs are removed unless every glob in the command misses.
    CshNullGlob,
    /// Runtime option state may select either behavior.
    Ambiguous,
}

impl ShellBehaviorAt<'_> {
    /// Returns the field-splitting behavior implied by the shell and runtime option state.
    pub fn field_splitting(&self) -> FieldSplittingBehavior {
        if self.shell != ShellDialect::Zsh {
            return FieldSplittingBehavior::UnquotedOnly;
        }

        match self
            .effective_zsh_options()
            .map(|options| options.sh_word_split)
        {
            Some(OptionValue::Off) => FieldSplittingBehavior::Never,
            Some(OptionValue::Unknown) => FieldSplittingBehavior::Ambiguous,
            Some(OptionValue::On) | None => FieldSplittingBehavior::UnquotedOnly,
        }
    }

    /// Returns the pathname-expansion behavior implied by the shell and runtime option state.
    pub fn pathname_expansion(&self) -> PathnameExpansionBehavior {
        if self.shell != ShellDialect::Zsh {
            return PathnameExpansionBehavior::SubstitutionResultsWhenUnquoted;
        }

        let Some(options) = self.effective_zsh_options() else {
            return PathnameExpansionBehavior::LiteralGlobsOnly;
        };

        match options.glob {
            OptionValue::Off => PathnameExpansionBehavior::Disabled,
            OptionValue::Unknown => match options.glob_subst {
                OptionValue::Off => PathnameExpansionBehavior::LiteralGlobsOnlyOrDisabled,
                OptionValue::On | OptionValue::Unknown => PathnameExpansionBehavior::Ambiguous,
            },
            OptionValue::On => match options.glob_subst {
                OptionValue::Off => PathnameExpansionBehavior::LiteralGlobsOnly,
                OptionValue::On => PathnameExpansionBehavior::SubstitutionResultsWhenUnquoted,
                OptionValue::Unknown => PathnameExpansionBehavior::Ambiguous,
            },
        }
    }

    /// Returns the unmatched-glob behavior implied by the shell and runtime option state.
    pub fn glob_failure(&self) -> GlobFailureBehavior {
        if self.shell != ShellDialect::Zsh {
            return GlobFailureBehavior::KeepLiteralOnNoMatch;
        }

        let Some(options) = self.effective_zsh_options() else {
            return GlobFailureBehavior::ErrorOnNoMatch;
        };

        match options.glob {
            OptionValue::Off => GlobFailureBehavior::KeepLiteralOnNoMatch,
            OptionValue::Unknown => GlobFailureBehavior::Ambiguous,
            OptionValue::On => match options.csh_null_glob {
                OptionValue::On => GlobFailureBehavior::CshNullGlob,
                OptionValue::Unknown => GlobFailureBehavior::Ambiguous,
                OptionValue::Off => match options.null_glob {
                    OptionValue::On => GlobFailureBehavior::DropUnmatchedPattern,
                    OptionValue::Unknown => GlobFailureBehavior::Ambiguous,
                    OptionValue::Off => match options.nomatch {
                        OptionValue::On => GlobFailureBehavior::ErrorOnNoMatch,
                        OptionValue::Off => GlobFailureBehavior::KeepLiteralOnNoMatch,
                        OptionValue::Unknown => GlobFailureBehavior::Ambiguous,
                    },
                },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SemanticBuildOptions, SemanticModel, ShellProfile};
    use shuck_indexer::Indexer;
    use shuck_parser::parser::Parser;

    fn model_with_profile(source: &str, profile: ShellProfile) -> SemanticModel {
        let output = Parser::with_profile(source, profile.clone())
            .parse()
            .unwrap();
        let indexer = Indexer::new(source, &output);
        SemanticModel::build_with_options(
            &output.file,
            source,
            &indexer,
            SemanticBuildOptions {
                shell_profile: Some(profile),
                ..SemanticBuildOptions::default()
            },
        )
    }

    #[test]
    fn tracks_field_splitting_by_offset() {
        for (source, expected) in [
            ("print $name\n", FieldSplittingBehavior::Never),
            (
                "setopt sh_word_split\nprint $name\n",
                FieldSplittingBehavior::UnquotedOnly,
            ),
            (
                "opt=sh_word_split\nsetopt \"$opt\"\nprint $name\n",
                FieldSplittingBehavior::Ambiguous,
            ),
        ] {
            let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
            let offset = source.find("print").expect("expected print offset");

            assert_eq!(
                model.shell_behavior_at(offset).field_splitting(),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn uses_non_zsh_default_behaviors() {
        let source = "printf '%s\\n' $name *.txt\n";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Bash));
        let offset = source.find("printf").expect("expected printf offset");
        let behavior = model.shell_behavior_at(offset);

        assert_eq!(
            behavior.field_splitting(),
            FieldSplittingBehavior::UnquotedOnly
        );
        assert_eq!(
            behavior.pathname_expansion(),
            PathnameExpansionBehavior::SubstitutionResultsWhenUnquoted
        );
        assert_eq!(
            behavior.glob_failure(),
            GlobFailureBehavior::KeepLiteralOnNoMatch
        );
    }

    #[test]
    fn tracks_pathname_expansion_by_offset() {
        for (source, expected) in [
            ("print $name\n", PathnameExpansionBehavior::LiteralGlobsOnly),
            (
                "setopt glob_subst\nprint $name\n",
                PathnameExpansionBehavior::SubstitutionResultsWhenUnquoted,
            ),
            (
                "setopt no_glob\nprint $name\n",
                PathnameExpansionBehavior::Disabled,
            ),
            (
                "opt=glob_subst\nsetopt \"$opt\"\nprint $name\n",
                PathnameExpansionBehavior::Ambiguous,
            ),
            (
                "if cond; then setopt no_glob; fi\nprint $name\n",
                PathnameExpansionBehavior::LiteralGlobsOnlyOrDisabled,
            ),
        ] {
            let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
            let offset = source.find("print").expect("expected print offset");

            assert_eq!(
                model.shell_behavior_at(offset).pathname_expansion(),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn pathname_expansion_respects_no_glob_precedence() {
        for (source, expected) in [
            (
                "setopt no_glob glob_subst\nprint $name\n",
                PathnameExpansionBehavior::Disabled,
            ),
            (
                "setopt glob_subst\nprint $name\n",
                PathnameExpansionBehavior::SubstitutionResultsWhenUnquoted,
            ),
            (
                "unsetopt glob_subst\nprint $name\n",
                PathnameExpansionBehavior::LiteralGlobsOnly,
            ),
        ] {
            let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
            let offset = source.find("print").expect("expected print offset");

            assert_eq!(
                model.shell_behavior_at(offset).pathname_expansion(),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn pathname_expansion_keeps_glob_subst_off_when_glob_is_flow_merged() {
        let source = "\
if cond; then
  setopt no_glob
fi
print $name
";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let offset = source.find("print").expect("expected print offset");
        let behavior = model.shell_behavior_at(offset).pathname_expansion();

        assert_eq!(
            behavior,
            PathnameExpansionBehavior::LiteralGlobsOnlyOrDisabled
        );
        assert!(behavior.literal_globs_can_expand());
        assert!(!behavior.unquoted_substitution_results_can_glob());
    }

    #[test]
    fn tracks_function_leaked_option_effects_by_offset() {
        let source = "\
fn() {
  setopt sh_word_split glob_subst
}
fn
print $name
";
        let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
        let offset = source.rfind("print").expect("expected print offset");

        assert_eq!(
            model.shell_behavior_at(offset).field_splitting(),
            FieldSplittingBehavior::UnquotedOnly
        );
        assert_eq!(
            model.shell_behavior_at(offset).pathname_expansion(),
            PathnameExpansionBehavior::SubstitutionResultsWhenUnquoted
        );
        assert_eq!(
            model.shell_behavior_at(offset).glob_failure(),
            GlobFailureBehavior::ErrorOnNoMatch
        );
    }

    #[test]
    fn tracks_glob_failure_modes_by_offset() {
        for (source, expected) in [
            ("print *\n", GlobFailureBehavior::ErrorOnNoMatch),
            (
                "setopt no_nomatch\nprint *\n",
                GlobFailureBehavior::KeepLiteralOnNoMatch,
            ),
            (
                "setopt null_glob\nprint *\n",
                GlobFailureBehavior::DropUnmatchedPattern,
            ),
            (
                "setopt csh_null_glob\nprint *\n",
                GlobFailureBehavior::CshNullGlob,
            ),
            (
                "opt=no_nomatch\nsetopt \"$opt\"\nprint *\n",
                GlobFailureBehavior::Ambiguous,
            ),
        ] {
            let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
            let offset = source.find("print").expect("expected print offset");

            assert_eq!(
                model.shell_behavior_at(offset).glob_failure(),
                expected,
                "{source}"
            );
        }
    }

    #[test]
    fn glob_failure_respects_option_precedence() {
        for (source, expected) in [
            (
                "setopt no_glob null_glob csh_null_glob\nprint *\n",
                GlobFailureBehavior::KeepLiteralOnNoMatch,
            ),
            (
                "setopt null_glob csh_null_glob\nprint *\n",
                GlobFailureBehavior::CshNullGlob,
            ),
            (
                "setopt null_glob\nunsetopt csh_null_glob\nprint *\n",
                GlobFailureBehavior::DropUnmatchedPattern,
            ),
            (
                "setopt no_nomatch null_glob csh_null_glob\nprint *\n",
                GlobFailureBehavior::CshNullGlob,
            ),
        ] {
            let model = model_with_profile(source, ShellProfile::native(ShellDialect::Zsh));
            let offset = source.find("print").expect("expected print offset");

            assert_eq!(
                model.shell_behavior_at(offset).glob_failure(),
                expected,
                "{source}"
            );
        }
    }
}
