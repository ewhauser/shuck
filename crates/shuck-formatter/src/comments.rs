use shuck_ast::Comment;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceComment<'a> {
    text: &'a str,
    line: usize,
    inline: bool,
}

impl<'a> SourceComment<'a> {
    #[must_use]
    pub fn text(&self) -> &'a str {
        self.text
    }
}

#[derive(Debug, Clone)]
pub struct Comments<'a> {
    items: Vec<SourceComment<'a>>,
    next: usize,
}

impl<'a> Comments<'a> {
    #[must_use]
    pub fn from_ast(source: &'a str, comments: &[Comment]) -> Self {
        let line_starts = line_starts(source);
        let mut items = Vec::with_capacity(comments.len());

        for comment in comments {
            let start = usize::from(comment.range.start());
            let end = usize::from(comment.range.end());
            if start >= end || end > source.len() {
                continue;
            }

            let line = line_number_for_offset(&line_starts, start);
            items.push(SourceComment {
                text: &source[start..end],
                line,
                inline: is_inline_comment(source, start),
            });
        }

        Self { items, next: 0 }
    }

    pub fn take_leading_before(&mut self, line: usize) -> Vec<SourceComment<'a>> {
        let mut taken = Vec::new();
        while let Some(comment) = self.items.get(self.next).copied() {
            if comment.line > line || (comment.line == line && comment.inline) {
                break;
            }
            taken.push(comment);
            self.next += 1;
        }
        taken
    }

    pub fn take_inline_for_line(&mut self, line: usize) -> Vec<SourceComment<'a>> {
        let mut taken = Vec::new();
        while let Some(comment) = self.items.get(self.next).copied() {
            if comment.line != line || !comment.inline {
                break;
            }
            taken.push(comment);
            self.next += 1;
        }
        taken
    }

    pub fn take_remaining(&mut self) -> Vec<SourceComment<'a>> {
        let remaining = self.items[self.next..].to_vec();
        self.next = self.items.len();
        remaining
    }
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (offset, byte) in source.bytes().enumerate() {
        if byte == b'\n' && offset + 1 < source.len() {
            starts.push(offset + 1);
        }
    }
    starts
}

fn line_number_for_offset(line_starts: &[usize], offset: usize) -> usize {
    match line_starts.binary_search(&offset) {
        Ok(index) => index + 1,
        Err(index) => index,
    }
}

fn is_inline_comment(source: &str, start: usize) -> bool {
    let line_start = source[..start].rfind('\n').map_or(0, |index| index + 1);
    source[line_start..start]
        .chars()
        .any(|character| !character.is_whitespace())
}
