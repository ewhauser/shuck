use super::*;

pub(super) fn parse_xargs_command<'a>(args: &[&'a Word], source: &str) -> XargsCommandFacts<'a> {
    let mut uses_null_input = false;
    let mut max_procs = None;
    let mut zero_digit_option_word = false;
    let mut inline_replace_options = Vec::new();
    let mut index = 0usize;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            if word_starts_with_literal_dash(word, source) {
                break;
            }
            break;
        };

        if text == "--" {
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        zero_digit_option_word |= text.contains('0');

        if let Some(long) = text.strip_prefix("--") {
            if long_name(long) == "null" {
                uses_null_input = true;
            }
            if long_name(long) == "max-procs"
                && let Some(argument) =
                    xargs_long_option_argument(long, args.get(index + 1), source)
            {
                max_procs = argument.parse::<u64>().ok();
            }

            let consume_next_argument = xargs_long_option_requires_separate_argument(long);
            index += 1;
            if consume_next_argument {
                index += 1;
            }
            continue;
        }

        let mut chars = text[1..].chars().peekable();
        let mut consume_next_argument = false;
        while let Some(flag) = chars.next() {
            if flag == '0' {
                uses_null_input = true;
            }
            if flag == 'i' {
                inline_replace_options.push(XargsInlineReplaceOptionFact {
                    span: word.span,
                    uses_default_replacement: chars.peek().is_none(),
                });
            }
            if flag == 'P' {
                let remainder = chars.collect::<String>();
                let has_inline_argument = !remainder.is_empty();
                let argument = if has_inline_argument {
                    Some(remainder)
                } else {
                    args.get(index + 1)
                        .and_then(|next| static_word_text(next, source))
                        .map(|value| value.into_owned())
                };
                max_procs = argument.and_then(|value| value.parse::<u64>().ok());
                consume_next_argument = !has_inline_argument;
                break;
            }

            match xargs_short_option_argument_style(flag) {
                XargsShortOptionArgumentStyle::None => {}
                XargsShortOptionArgumentStyle::OptionalInlineOnly => break,
                XargsShortOptionArgumentStyle::Required => {
                    if chars.peek().is_none() {
                        consume_next_argument = true;
                    }
                    break;
                }
            }
        }

        index += 1;
        if consume_next_argument {
            index += 1;
        }
    }

    XargsCommandFacts {
        uses_null_input,
        max_procs,
        zero_digit_option_word,
        inline_replace_options: inline_replace_options.into_boxed_slice(),
        command_operand_words: args[index..].to_vec().into_boxed_slice(),
        sc2267_default_replace_silent_shape: xargs_sc2267_default_replace_silent_shape(
            &args[index..],
            source,
        ),
    }
}

fn xargs_sc2267_default_replace_silent_shape(args: &[&Word], source: &str) -> bool {
    xargs_command_is_shell_c_wrapper(args, source)
        || xargs_command_is_echo_leading_dash_replacement(args, source)
}

fn xargs_command_is_shell_c_wrapper(args: &[&Word], source: &str) -> bool {
    let args = if args
        .first()
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("command")
    {
        &args[1..]
    } else {
        args
    };

    let Some(command_name) = args.first().and_then(|word| static_word_text(word, source)) else {
        return false;
    };

    matches!(
        command_basename(command_name.as_ref()),
        "sh" | "bash" | "dash" | "ksh" | "zsh"
    ) && args
        .get(1)
        .and_then(|word| static_word_text(word, source))
        .as_deref()
        == Some("-c")
}

fn xargs_command_is_echo_leading_dash_replacement(args: &[&Word], source: &str) -> bool {
    let Some(command_name) = args.first().and_then(|word| static_word_text(word, source)) else {
        return false;
    };

    if command_basename(command_name.as_ref()) != "echo" {
        return false;
    }

    let Some(first_operand) = args.get(1) else {
        return false;
    };
    let literal_prefix = leading_literal_word_prefix(first_operand, source);
    literal_prefix.starts_with('-') && literal_prefix != "-" && literal_prefix.contains("{}")
}

fn command_basename(name: &str) -> &str {
    name.rsplit('/').next().unwrap_or(name)
}

fn xargs_long_option_argument(
    option: &str,
    next_word: Option<&&Word>,
    source: &str,
) -> Option<String> {
    if let Some((_, value)) = option.split_once('=') {
        return Some(value.to_owned());
    }

    next_word
        .and_then(|word| static_word_text(word, source))
        .map(|value| value.into_owned())
}

fn long_name(option: &str) -> &str {
    option.split_once('=').map_or(option, |(name, _)| name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XargsShortOptionArgumentStyle {
    None,
    OptionalInlineOnly,
    Required,
}

pub(crate) fn xargs_short_option_argument_style(flag: char) -> XargsShortOptionArgumentStyle {
    match flag {
        'e' | 'i' | 'l' => XargsShortOptionArgumentStyle::OptionalInlineOnly,
        'a' | 'E' | 'I' | 'L' | 'n' | 'P' | 's' | 'd' => XargsShortOptionArgumentStyle::Required,
        _ => XargsShortOptionArgumentStyle::None,
    }
}

pub(crate) fn xargs_long_option_requires_separate_argument(option: &str) -> bool {
    if option.contains('=') {
        return false;
    }

    matches!(
        option,
        "arg-file"
            | "delimiter"
            | "max-args"
            | "max-chars"
            | "max-lines"
            | "max-procs"
            | "process-slot-var"
    )
}
