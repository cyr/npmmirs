use std::path::PathBuf;

use crate::CliOpts;

pub mod manifest;

pub fn local_metadata_path(opts: &CliOpts, package: &str) -> PathBuf {
    let output_base = opts.output.strip_suffix('/').unwrap_or(&opts.output);

    PathBuf::from(format!("{output_base}/{package}/index.json"))
}