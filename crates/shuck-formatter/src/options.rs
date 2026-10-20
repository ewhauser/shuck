use std::path::Path;

use shuck_parser::ShellDialect as ParseDialect;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IndentStyle {
    #[default]
    Tab,
    Space,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LineEnding {
    #[default]
    Lf,
    CrLf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellDialect {
    #[default]
    Auto,
    Bash,
    Posix,
    Mksh,
    Zsh,
}

impl ShellDialect {
    #[must_use]
    pub fn resolve(self, source: &str, path: Option<&Path>) -> ParseDialect {
        match self {
            Self::Auto => infer_dialect(source, path),
            Self::Bash => ParseDialect::Bash,
            Self::Posix => ParseDialect::Posix,
            Self::Mksh => ParseDialect::Mksh,
            Self::Zsh => ParseDialect::Zsh,
        }
    }

    /// Maps a linter shell name (`sh`, `bash`, `dash`, `mksh`, `zsh`) onto the
    /// formatter dialect that handles it. Returns `None` for shells the
    /// formatter has no dedicated support for, such as `ksh`.
    #[must_use]
    pub fn for_shell_name(name: &str) -> Option<Self> {
        match name {
            "sh" | "dash" => Some(Self::Posix),
            "bash" => Some(Self::Bash),
            "mksh" => Some(Self::Mksh),
            "zsh" => Some(Self::Zsh),
            _ => None,
        }
    }
}

/// Partial formatter settings supplied by CLI flags, config files, or editor
/// options. Unset fields leave the existing option untouched.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FormatSettingsPatch {
    /// Optional dialect override.
    pub dialect: Option<ShellDialect>,
    /// Optional indentation style override.
    pub indent_style: Option<IndentStyle>,
    /// Optional indentation width override.
    pub indent_width: Option<u8>,
    /// Optional binary-next-line override.
    pub binary_next_line: Option<bool>,
    /// Optional switch-case indentation override.
    pub switch_case_indent: Option<bool>,
    /// Optional spaced-redirection override.
    pub space_redirects: Option<bool>,
    /// Optional padding-preservation override.
    pub keep_padding: Option<bool>,
    /// Optional function-next-line override.
    pub function_next_line: Option<bool>,
    /// Optional never-split override.
    pub never_split: Option<bool>,
    /// Optional simplification override.
    pub simplify: Option<bool>,
    /// Optional minification override.
    pub minify: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellFormatOptions {
    dialect: ShellDialect,
    indent_style: IndentStyle,
    indent_width: u8,
    binary_next_line: bool,
    switch_case_indent: bool,
    space_redirects: bool,
    keep_padding: bool,
    function_next_line: bool,
    never_split: bool,
    simplify: bool,
    minify: bool,
}

impl Default for ShellFormatOptions {
    fn default() -> Self {
        Self {
            dialect: ShellDialect::Auto,
            indent_style: IndentStyle::Tab,
            indent_width: 8,
            binary_next_line: false,
            switch_case_indent: false,
            space_redirects: false,
            keep_padding: false,
            function_next_line: false,
            never_split: false,
            simplify: false,
            minify: false,
        }
    }
}

macro_rules! option_getters {
    ($($method:ident: $field:ident -> $ty:ty;)+) => {
        $(
            #[must_use]
            pub fn $method(&self) -> $ty {
                self.$field
            }
        )+
    };
}

macro_rules! option_builders {
    ($($method:ident: $field:ident -> $ty:ty;)+) => {
        $(
            #[must_use]
            pub fn $method(mut self, value: $ty) -> Self {
                self.$field = value;
                self
            }
        )+
    };
}

macro_rules! resolved_option_getters {
    ($($method:ident: $field:ident -> $ty:ty;)+) => {
        $(
            #[must_use]
            pub fn $method(&self) -> $ty {
                self.options.$field
            }
        )+
    };
}

impl ShellFormatOptions {
    option_getters! {
        dialect: dialect -> ShellDialect;
        indent_style: indent_style -> IndentStyle;
        indent_width: indent_width -> u8;
        binary_next_line: binary_next_line -> bool;
        switch_case_indent: switch_case_indent -> bool;
        space_redirects: space_redirects -> bool;
        keep_padding: keep_padding -> bool;
        function_next_line: function_next_line -> bool;
        never_split: never_split -> bool;
        simplify: simplify -> bool;
        minify: minify -> bool;
    }

    option_builders! {
        with_dialect: dialect -> ShellDialect;
        with_indent_style: indent_style -> IndentStyle;
        with_binary_next_line: binary_next_line -> bool;
        with_switch_case_indent: switch_case_indent -> bool;
        with_space_redirects: space_redirects -> bool;
        with_keep_padding: keep_padding -> bool;
        with_function_next_line: function_next_line -> bool;
        with_never_split: never_split -> bool;
        with_simplify: simplify -> bool;
        with_minify: minify -> bool;
    }

    #[must_use]
    pub fn with_indent_width(mut self, indent_width: u8) -> Self {
        self.indent_width = indent_width.max(1);
        self
    }

    /// Applies every field set in `patch` on top of these options.
    #[must_use]
    pub fn with_patch(mut self, patch: &FormatSettingsPatch) -> Self {
        if let Some(dialect) = patch.dialect {
            self = self.with_dialect(dialect);
        }
        if let Some(indent_style) = patch.indent_style {
            self = self.with_indent_style(indent_style);
        }
        if let Some(indent_width) = patch.indent_width {
            self = self.with_indent_width(indent_width);
        }
        if let Some(binary_next_line) = patch.binary_next_line {
            self = self.with_binary_next_line(binary_next_line);
        }
        if let Some(switch_case_indent) = patch.switch_case_indent {
            self = self.with_switch_case_indent(switch_case_indent);
        }
        if let Some(space_redirects) = patch.space_redirects {
            self = self.with_space_redirects(space_redirects);
        }
        if let Some(keep_padding) = patch.keep_padding {
            self = self.with_keep_padding(keep_padding);
        }
        if let Some(function_next_line) = patch.function_next_line {
            self = self.with_function_next_line(function_next_line);
        }
        if let Some(never_split) = patch.never_split {
            self = self.with_never_split(never_split);
        }
        if let Some(simplify) = patch.simplify {
            self = self.with_simplify(simplify);
        }
        if let Some(minify) = patch.minify {
            self = self.with_minify(minify);
        }
        self
    }

    #[must_use]
    pub fn resolve(&self, source: &str, path: Option<&Path>) -> ResolvedShellFormatOptions {
        let mut resolved = self.resolve_for_format(source, path);
        resolved.line_ending = line_ending_from_source_index(source);
        resolved
    }

    pub(crate) fn resolve_for_format(
        &self,
        source: &str,
        path: Option<&Path>,
    ) -> ResolvedShellFormatOptions {
        let mut options = self.clone();
        options.indent_width = options.indent_width.max(1);
        ResolvedShellFormatOptions {
            dialect: self.dialect.resolve(source, path),
            options,
            line_ending: LineEnding::Lf,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedShellFormatOptions {
    options: ShellFormatOptions,
    dialect: ParseDialect,
    line_ending: LineEnding,
}

impl ResolvedShellFormatOptions {
    option_getters! {
        dialect: dialect -> ParseDialect;
    }

    resolved_option_getters! {
        indent_style: indent_style -> IndentStyle;
        indent_width: indent_width -> u8;
    }

    pub(crate) fn indent_unit_columns(&self) -> usize {
        match self.indent_style() {
            IndentStyle::Tab => 1,
            IndentStyle::Space => usize::from(self.indent_width()),
        }
    }

    pub(crate) fn indent_columns(&self, levels: usize) -> usize {
        levels * self.indent_unit_columns()
    }

    pub(crate) fn push_indent_units(&self, target: &mut String, levels: usize) {
        self.push_indent_columns(target, self.indent_columns(levels));
    }

    pub(crate) fn push_indent_columns(&self, target: &mut String, columns: usize) {
        let ch = match self.indent_style() {
            IndentStyle::Tab => '\t',
            IndentStyle::Space => ' ',
        };
        target.extend(std::iter::repeat_n(ch, columns));
    }

    pub(crate) fn indent_prefix(&self, levels: usize) -> String {
        let mut prefix = String::new();
        self.push_indent_units(&mut prefix, levels);
        prefix
    }

    resolved_option_getters! {
        binary_next_line: binary_next_line -> bool;
        switch_case_indent: switch_case_indent -> bool;
        space_redirects: space_redirects -> bool;
        keep_padding: keep_padding -> bool;
        function_next_line: function_next_line -> bool;
        never_split: never_split -> bool;
        simplify: simplify -> bool;
        minify: minify -> bool;
    }

    #[must_use]
    pub fn compact_layout(&self) -> bool {
        self.minify() || self.never_split()
    }

    option_getters! {
        line_ending: line_ending -> LineEnding;
    }

    pub(crate) fn with_line_ending(mut self, line_ending: LineEnding) -> Self {
        self.line_ending = line_ending;
        self
    }
}

fn line_ending_from_source_index(source: &str) -> LineEnding {
    match shuck_indexer::LineIndex::new(source).line_ending() {
        shuck_indexer::LineEndingStyle::Lf => LineEnding::Lf,
        shuck_indexer::LineEndingStyle::CrLf => LineEnding::CrLf,
    }
}

fn infer_dialect(source: &str, path: Option<&Path>) -> ParseDialect {
    if let Some(first_line) = source.lines().next()
        && let Some(interpreter) = shuck_parser::shebang::interpreter_name(first_line)
    {
        return ParseDialect::from_name(interpreter);
    }

    path.and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(ParseDialect::from_name)
        .unwrap_or(ParseDialect::Bash)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn zsh_extension_resolves_to_zsh_dialect() {
        let resolved = ShellFormatOptions::default()
            .resolve("print ${(m)name}\n", Some(Path::new("script.zsh")));

        assert_eq!(resolved.dialect(), ParseDialect::Zsh);
    }

    #[test]
    fn zsh_shebang_resolves_to_zsh_dialect() {
        let resolved = ShellFormatOptions::default().resolve(
            "#!/bin/zsh\nprint ${(m)name}\n",
            Some(Path::new("script.sh")),
        );

        assert_eq!(resolved.dialect(), ParseDialect::Zsh);
    }

    #[test]
    fn explicit_zsh_dialect_overrides_path_inference() {
        let options = ShellFormatOptions::default().with_dialect(ShellDialect::Zsh);
        let resolved = options.resolve("print ${(m)name}\n", Some(Path::new("script.sh")));

        assert_eq!(resolved.dialect(), ParseDialect::Zsh);
    }
}
