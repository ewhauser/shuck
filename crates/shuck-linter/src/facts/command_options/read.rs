use super::*;

pub(super) fn read_uses_raw_input(args: &[&Word], source: &str) -> bool {
    let mut index = 0usize;
    let mut pending_dynamic_option_arg = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                pending_dynamic_option_arg = true;
                index += 1;
                continue;
            }

            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        };

        if text == "--" {
            break;
        }

        if !text.starts_with('-') || text == "-" {
            if pending_dynamic_option_arg {
                pending_dynamic_option_arg = false;
                index += 1;
                continue;
            }

            break;
        }

        pending_dynamic_option_arg = false;
        let mut chars = text[1..].chars().peekable();
        while let Some(flag) = chars.next() {
            if flag == 'r' {
                return true;
            }

            if option_takes_argument(flag) {
                if chars.peek().is_none() {
                    index += 1;
                }
                break;
            }
        }

        index += 1;
    }

    false
}

pub(super) fn read_target_name_uses(
    args: &[&Word],
    semantic: &LinterSemanticArtifacts<'_>,
    source: &str,
) -> Box<[ComparableNameUse]> {
    read_name_uses(args, semantic, source).0
}

pub(super) fn read_array_target_name_uses(
    args: &[&Word],
    semantic: &LinterSemanticArtifacts<'_>,
    source: &str,
) -> Box<[ComparableNameUse]> {
    read_name_uses(args, semantic, source).1
}

fn read_name_uses(
    args: &[&Word],
    semantic: &LinterSemanticArtifacts<'_>,
    source: &str,
) -> (Box<[ComparableNameUse]>, Box<[ComparableNameUse]>) {
    let mut targets = Vec::new();
    let mut array_targets = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                return (Vec::new().into_boxed_slice(), Vec::new().into_boxed_slice());
            }

            for target in &args[index..] {
                targets.extend(comparable_read_target_name_uses(target, semantic, source));
            }
            break;
        };

        if text == "--" {
            for target in &args[index + 1..] {
                targets.extend(comparable_read_target_name_uses(target, semantic, source));
            }
            break;
        }

        if !text.starts_with('-') || text == "-" {
            for target in &args[index..] {
                targets.extend(comparable_read_target_name_uses(target, semantic, source));
            }
            break;
        }

        let mut chars = text[1..].char_indices().peekable();
        let mut saw_array_target = false;
        while let Some((flag_offset, flag)) = chars.next() {
            if flag == 'a' {
                let attached_start = flag_offset + 2;
                if attached_start < text.len() {
                    if let Some(target) =
                        read_attached_array_target_name_use(word, source, &text[attached_start..])
                    {
                        array_targets.push(target.clone());
                        targets.push(target);
                    }
                } else if let Some(target) = args.get(index + 1) {
                    let target_uses = comparable_read_target_name_uses(target, semantic, source);
                    array_targets.extend(target_uses.iter().cloned());
                    targets.extend(target_uses);
                    index += 1;
                }
                saw_array_target = true;
                break;
            }

            if option_takes_argument(flag) {
                if chars.peek().is_none() {
                    index += 1;
                }
                break;
            }
        }

        if saw_array_target {
            break;
        }

        index += 1;
    }

    (targets.into_boxed_slice(), array_targets.into_boxed_slice())
}

fn read_attached_array_target_name_use(
    word: &Word,
    source: &str,
    target_text: &str,
) -> Option<ComparableNameUse> {
    if !comparable_name_text(target_text) {
        return None;
    }

    let target_span = word
        .span
        .slice(source)
        .rfind(target_text)
        .map(|start| {
            read_option_attached_target_span(word.span, source, start, start + target_text.len())
        })
        .unwrap_or(word.span);

    Some(ComparableNameUse {
        span: target_span,
        key: ComparableNameKey(target_text.into()),
        kind: ComparableNameUseKind::Literal,
    })
}

fn read_option_attached_target_span(span: Span, source: &str, start: usize, end: usize) -> Span {
    let start_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + start]);
    let end_pos = span
        .start
        .advanced_by(&source[span.start.offset..span.start.offset + end]);
    Span::from_positions(start_pos, end_pos)
}

fn option_takes_argument(flag: char) -> bool {
    matches!(flag, 'a' | 'd' | 'i' | 'n' | 'N' | 'p' | 't' | 'u')
}
