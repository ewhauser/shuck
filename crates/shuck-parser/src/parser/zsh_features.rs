use super::*;

impl<'a> Parser<'a> {
    pub(super) fn zsh_options_at_offset(&self, offset: usize) -> Option<&ZshOptionState> {
        self.zsh_timeline
            .as_ref()
            .map(|timeline| timeline.options_at(offset))
            .or_else(|| self.shell_profile.zsh_options())
    }

    pub(super) fn current_zsh_options(&self) -> Option<&ZshOptionState> {
        self.zsh_options_at_offset(self.current_span.start.offset)
    }

    pub(super) fn zsh_short_loops_enabled(&self) -> bool {
        self.dialect.features().zsh_foreach_loop
            && !self
                .current_zsh_options()
                .is_some_and(|options| options.short_loops.is_definitely_off())
    }

    pub(super) fn zsh_short_repeat_enabled(&self) -> bool {
        self.dialect.features().zsh_repeat_loop
            && !self
                .current_zsh_options()
                .is_some_and(|options| options.short_repeat.is_definitely_off())
    }

    pub(super) fn zsh_brace_bodies_enabled(&self) -> bool {
        self.dialect.features().zsh_brace_if
            && !self
                .current_zsh_options()
                .is_some_and(|options| options.ignore_braces.is_definitely_on())
    }

    pub(super) fn zsh_brace_if_enabled(&self) -> bool {
        self.zsh_brace_bodies_enabled()
    }

    pub(super) fn zsh_glob_parse_features_at(&self, offset: usize) -> ZshGlobParseFeatures {
        let options = self.zsh_options_at_offset(offset);
        let is_zsh = self.dialect == ShellDialect::Zsh;
        ZshGlobParseFeatures {
            classic_qualifiers: self.dialect.features().zsh_glob_qualifiers
                && !options.is_some_and(|options| {
                    options.ignore_braces.is_definitely_on()
                        || options.bare_glob_qual.is_definitely_off()
                }),
            extended_glob: is_zsh
                && !options.is_some_and(|options| options.extended_glob.is_definitely_off()),
            // Preserve existing non-zsh pattern-group parsing behavior.
            ksh_groups: !is_zsh
                || !options.is_some_and(|options| options.ksh_glob.is_definitely_off()),
            bare_groups: is_zsh
                && !options.is_some_and(|options| options.sh_glob.is_definitely_on()),
        }
    }

    pub(super) fn zsh_glob_word_parsing_enabled_at(&self, offset: usize) -> bool {
        self.dialect == ShellDialect::Zsh
            && self
                .zsh_glob_parse_features_at(offset)
                .zsh_word_parsing_enabled()
    }

    pub(super) fn brace_syntax_enabled_at(&self, offset: usize) -> bool {
        !self.zsh_options_at_offset(offset).is_some_and(|options| {
            options.ignore_braces.is_definitely_on()
                || options.ignore_close_braces.is_definitely_on()
        })
    }

    pub(super) fn brace_ccl_enabled_at(&self, offset: usize) -> bool {
        self.zsh_options_at_offset(offset)
            .is_some_and(|options| options.brace_ccl.is_definitely_on())
    }
}
