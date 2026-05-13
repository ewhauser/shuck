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
    pub fn apply_changes(
        &mut self,
        changes: Vec<TextDocumentContentChangeEvent>,
        new_version: DocumentVersion,
        encoding: PositionEncoding,
    ) {
        if let [
            TextDocumentContentChangeEvent {
                range: None, text, ..
            },
        ] = changes.as_slice()
        {
            self.contents.clone_from(text);
            self.index = LineIndex::new(&self.contents);
            self.version = new_version;
            return;
        }

        let mut new_contents = self.contents.clone();
        let mut active_index = self.index.clone();

        for change in changes {
            if let Some(range) = change.range {
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
    }

    /// Update the document version without changing contents.
    pub fn update_version(&mut self, new_version: DocumentVersion) {
        debug_assert!(new_version >= self.version);
        self.version = new_version;
    }
}
