use crate::facts::words::FactWordRef;

pub(crate) struct ParsedTrapArgs<'a> {
    pub(crate) signal_words: &'a [FactWordRef<'a>],
    pub(crate) listing_mode: bool,
}

pub(crate) fn parse_trap_args<'a>(
    args: &'a [FactWordRef<'a>],
    source: &'a str,
) -> Option<ParsedTrapArgs<'a>> {
    let first = args.first().and_then(|word| word.static_text(source));

    match first.as_deref() {
        Some(first) if trap_is_listing_mode(first) => Some(ParsedTrapArgs {
            signal_words: &args[1..],
            listing_mode: true,
        }),
        Some("--") => {
            let signal_words = match args.len() {
                0 | 1 => return None,
                2 => &args[1..],
                _ => &args[2..],
            };

            Some(ParsedTrapArgs {
                signal_words,
                listing_mode: false,
            })
        }
        _ => {
            let signal_words = match args.len() {
                0 => return None,
                1 => args,
                _ => &args[1..],
            };

            Some(ParsedTrapArgs {
                signal_words,
                listing_mode: false,
            })
        }
    }
}

fn trap_is_listing_mode(text: &str) -> bool {
    text.strip_prefix('-').is_some_and(|flags| {
        !flags.is_empty() && flags.chars().all(|flag| matches!(flag, 'l' | 'p'))
    })
}
