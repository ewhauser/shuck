use lsp_types::TextDocumentContentChangeEvent;
use shuck_indexer::LineIndex;

use crate::PositionEncoding;

use super::RangeExt;

pub(crate) type DocumentVersion = i32;

/// In-memory representation of an open LSP text document.
#[derive(Debug, Clone)]
pub struct TextDocument {
    contents: String,
    index: LineIndex,
    version: DocumentVersion,
    language_id: Option<LanguageId>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum LanguageId {
    Bash,
    ShellScript,
    Sh,
    Zsh,
    Ksh,
    Other,
}

impl From<&str> for LanguageId {
    fn from(language_id: &str) -> Self {
        match language_id {
            "bash" => Self::Bash,
            "shellscript" => Self::ShellScript,
            "sh" => Self::Sh,
            "zsh" => Self::Zsh,
            "ksh" => Self::Ksh,
            _ => Self::Other,
        }
    }
}

impl TextDocument {
    /// Create a document with initial contents and version.
    pub fn new(contents: String, version: DocumentVersion) -> Self {
        let index = LineIndex::new(&contents);
        Self {
            contents,
            index,
            version,
            language_id: None,
        }
    }

    /// Return a copy with the LSP language identifier attached.
    #[must_use]
    pub fn with_language_id(mut self, language_id: &str) -> Self {
        self.language_id = Some(LanguageId::from(language_id));
        self
    }

    /// Return the current document contents.
    pub fn contents(&self) -> &str {
        &self.contents
    }

    /// Return the current line index for the document.
    pub fn index(&self) -> &LineIndex {
        &self.index
    }

    /// Return the current LSP document version.
    pub fn version(&self) -> DocumentVersion {
        self.version
    }

    /// Return the parsed language identifier, if one was supplied by the client.
    pub fn language_id(&self) -> Option<LanguageId> {
        self.language_id
    }

    /// Apply LSP content changes and update the document version.
    ///
    /// The update is all-or-nothing: a version older than the current one or
    /// a change whose range ends before it starts is rejected and leaves the
    /// document untouched, rather than guessing at what the client meant.
    pub fn apply_changes(
        &mut self,
        changes: Vec<TextDocumentContentChangeEvent>,
        new_version: DocumentVersion,
        encoding: PositionEncoding,
    ) -> crate::Result<()> {
        if new_version < self.version {
            anyhow::bail!(
                "rejecting out-of-order document change: version {new_version} is older than {}",
                self.version
            );
        }

        if let [
            TextDocumentContentChangeEvent {
                range: None, text, ..
            },
        ] = changes.as_slice()
        {
            self.contents.clone_from(text);
            self.index = LineIndex::new(&self.contents);
            self.version = new_version;
            return Ok(());
        }

        let mut new_contents = self.contents.clone();
        let mut active_index = self.index.clone();

        for change in changes {
            if let Some(range) = change.range {
                if (range.start.line, range.start.character) > (range.end.line, range.end.character)
                {
                    anyhow::bail!(
                        "rejecting document change with reversed range {}:{}..{}:{}",
                        range.start.line,
                        range.start.character,
                        range.end.line,
                        range.end.character
                    );
                }
                let range = range.to_text_range(&new_contents, &active_index, encoding);
                new_contents.replace_range(
                    usize::from(range.start())..usize::from(range.end()),
                    &change.text,
                );
            } else {
                new_contents = change.text;
            }
            active_index = LineIndex::new(&new_contents);
        }

        self.contents = new_contents;
        self.index = active_index;
        self.version = new_version;
        Ok(())
    }

    /// Update the document version without changing contents.
    ///
    /// Older versions are ignored so the document never moves backwards.
    pub fn update_version(&mut self, new_version: DocumentVersion) {
        if new_version >= self.version {
            self.version = new_version;
        }
    }
}

#[cfg(test)]
mod tests {
    use lsp_types::{Position, Range};

    use super::*;

    fn ranged_change(
        start: (u32, u32),
        end: (u32, u32),
        text: &str,
    ) -> TextDocumentContentChangeEvent {
        TextDocumentContentChangeEvent {
            range: Some(Range::new(
                Position::new(start.0, start.1),
                Position::new(end.0, end.1),
            )),
            range_length: None,
            text: text.to_owned(),
        }
    }

    #[test]
    fn rejects_changes_with_an_older_version() {
        let mut document = TextDocument::new("echo a\n".to_owned(), 5);

        let result = document.apply_changes(
            vec![ranged_change((0, 5), (0, 6), "b")],
            4,
            PositionEncoding::UTF16,
        );

        assert!(result.is_err());
        assert_eq!(document.contents(), "echo a\n");
        assert_eq!(document.version(), 5);
    }

    #[test]
    fn rejects_reversed_ranges_without_partial_application() {
        let mut document = TextDocument::new("echo a\necho b\n".to_owned(), 1);

        let result = document.apply_changes(
            vec![
                ranged_change((0, 5), (0, 6), "x"),
                ranged_change((1, 6), (1, 5), "y"),
            ],
            2,
            PositionEncoding::UTF16,
        );

        assert!(result.is_err());
        assert_eq!(document.contents(), "echo a\necho b\n");
        assert_eq!(document.version(), 1);
    }

    #[test]
    fn appending_past_crlf_line_end_keeps_the_terminator_intact() {
        let mut document = TextDocument::new("echo a\r\n".to_owned(), 1);

        document
            .apply_changes(
                vec![ranged_change((0, 50), (0, 50), " b")],
                2,
                PositionEncoding::UTF16,
            )
            .expect("change should apply");

        assert_eq!(document.contents(), "echo a b\r\n");
    }

    #[test]
    fn update_version_never_moves_backwards() {
        let mut document = TextDocument::new(String::new(), 3);
        document.update_version(2);
        assert_eq!(document.version(), 3);
        document.update_version(4);
        assert_eq!(document.version(), 4);
    }
}
