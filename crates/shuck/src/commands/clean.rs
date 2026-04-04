use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

use anyhow::Result;
use shuck_cache::cache_dir;

use crate::ExitStatus;
use crate::args::CleanCommand;
use crate::config::resolve_project_root_for_input;

pub(crate) fn clean(args: CleanCommand) -> Result<ExitStatus> {
    let cwd = std::env::current_dir()?;
    let inputs = if args.paths.is_empty() {
        vec![cwd.clone()]
    } else {
        args.paths
            .iter()
            .map(|path| {
                if path.is_absolute() {
                    path.clone()
                } else {
                    cwd.join(path)
                }
            })
            .collect::<Vec<PathBuf>>()
    };

    let mut roots = BTreeSet::new();
    for input in inputs {
        roots.insert(resolve_project_root_for_input(&input)?);
    }

    for root in roots {
        match fs::remove_dir_all(cache_dir(&root)) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }

    let mut stdout = BufWriter::new(io::stdout().lock());
    writeln!(stdout, "cache cleared")?;

    Ok(ExitStatus::Success)
}
