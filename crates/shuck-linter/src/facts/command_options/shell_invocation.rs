use super::*;

pub(super) fn parse_ssh_command(args: &[&Word], source: &str) -> Option<SshCommandFacts> {
    let remote_args = ssh_remote_args(args, source)?;
    let (last_remote_arg, leading_remote_args) = remote_args.split_last()?;
    if leading_remote_args
        .iter()
        .any(|word| word_starts_with_static_or_literal_dash(word, source))
    {
        return None;
    }

    let local_expansion_spans = last_remote_arg
        .is_fully_double_quoted()
        .then(|| {
            double_quoted_expansion_part_spans(last_remote_arg)
                .into_iter()
                .next()
        })
        .flatten()
        .into_iter()
        .collect::<Vec<_>>();

    (!local_expansion_spans.is_empty()).then_some(SshCommandFacts {
        local_expansion_spans: local_expansion_spans.into_boxed_slice(),
    })
}

pub(super) fn parse_su_command(args: &[&Word], source: &str) -> SuCommandFacts {
    let mut pending_option_arg = false;
    for word in args {
        let Some(text) = static_word_text(word, source) else {
            if pending_option_arg {
                pending_option_arg = false;
            }
            continue;
        };

        if pending_option_arg {
            pending_option_arg = false;
            continue;
        }

        match text.as_ref() {
            "-" | "-l" | "--login" => {
                return SuCommandFacts {
                    has_login_flag: true,
                };
            }
            "--" => {
                break;
            }
            _ if su_long_option_takes_argument(text.as_ref()) => {
                pending_option_arg = true;
                continue;
            }
            _ => {}
        }

        if text.starts_with("--") {
            continue;
        }

        if !text.starts_with('-') {
            continue;
        }

        let mut flags = text[1..].chars().peekable();
        while let Some(flag) = flags.next() {
            match flag {
                'l' => {
                    return SuCommandFacts {
                        has_login_flag: true,
                    };
                }
                flag if su_short_option_takes_argument(flag) => {
                    if flags.peek().is_none() {
                        pending_option_arg = true;
                    }
                    break;
                }
                _ => {}
            }
        }
    }

    SuCommandFacts {
        has_login_flag: false,
    }
}

fn su_long_option_takes_argument(text: &str) -> bool {
    matches!(
        text,
        "--command" | "--group" | "--shell" | "--supp-group" | "--whitelist-environment"
    )
}

fn su_short_option_takes_argument(flag: char) -> bool {
    matches!(flag, 'C' | 'c' | 'g' | 'G' | 's' | 'w')
}

fn ssh_remote_args<'a>(args: &'a [&'a Word], source: &str) -> Option<&'a [&'a Word]> {
    let mut index = 0usize;
    let mut saw_local_option = false;

    while let Some(word) = args.get(index) {
        let Some(text) = static_word_text(word, source) else {
            break;
        };

        if text == "--" {
            saw_local_option = true;
            index += 1;
            break;
        }

        if !text.starts_with('-') || text == "-" {
            break;
        }

        saw_local_option = true;
        let consumes_next = ssh_option_consumes_next_argument(text.as_ref())?;
        index += 1;
        if consumes_next {
            args.get(index)?;
            index += 1;
        }
    }

    if saw_local_option {
        return None;
    }

    let _destination = args.get(index)?;
    Some(&args[index + 1..])
}

pub(crate) fn ssh_option_consumes_next_argument(text: &str) -> Option<bool> {
    if !text.starts_with('-') || text == "-" {
        return Some(false);
    }
    if text == "--" {
        return Some(false);
    }

    let bytes = text.as_bytes();
    let mut index = 1usize;
    while index < bytes.len() {
        let flag = bytes[index];
        if ssh_option_requires_argument(flag) {
            return Some(index + 1 == bytes.len());
        }
        if !ssh_option_is_flag(flag) {
            return None;
        }
        index += 1;
    }

    Some(false)
}

fn ssh_option_requires_argument(flag: u8) -> bool {
    matches!(
        flag,
        b'B' | b'b'
            | b'c'
            | b'D'
            | b'E'
            | b'e'
            | b'F'
            | b'I'
            | b'i'
            | b'J'
            | b'L'
            | b'l'
            | b'm'
            | b'O'
            | b'o'
            | b'p'
            | b'P'
            | b'Q'
            | b'R'
            | b'S'
            | b'W'
            | b'w'
    )
}

fn ssh_option_is_flag(flag: u8) -> bool {
    ssh_option_requires_argument(flag)
        || matches!(
            flag,
            b'4' | b'6'
                | b'A'
                | b'a'
                | b'C'
                | b'f'
                | b'G'
                | b'g'
                | b'K'
                | b'k'
                | b'M'
                | b'N'
                | b'n'
                | b'q'
                | b's'
                | b'T'
                | b't'
                | b'V'
                | b'v'
                | b'X'
                | b'x'
                | b'Y'
                | b'y'
        )
}

pub(crate) fn shell_flag_contains_command_string(flag: &str) -> bool {
    let Some(cluster) = flag.strip_prefix('-') else {
        return false;
    };
    !cluster.is_empty()
        && !cluster.starts_with('-')
        && cluster.bytes().all(shell_short_flag_is_clusterable)
        && cluster.bytes().any(|byte| byte == b'c')
}

fn shell_short_flag_is_clusterable(flag: u8) -> bool {
    matches!(
        flag,
        b'a' | b'b'
            | b'c'
            | b'e'
            | b'f'
            | b'h'
            | b'i'
            | b'k'
            | b'l'
            | b'm'
            | b'n'
            | b'p'
            | b'r'
            | b's'
            | b't'
            | b'u'
            | b'v'
            | b'x'
    )
}
