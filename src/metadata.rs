use std::path::PathBuf;

use crate::CliOpts;

pub mod manifest;
pub mod package_index;
pub mod sparse_metadata;

pub fn local_metadata_path(opts: &CliOpts, package: &str) -> PathBuf {
    let output_base = opts.output.strip_suffix('/').unwrap_or(&opts.output);

    PathBuf::from(format!("{output_base}/{package}/index.json"))
}

pub fn local_metadata_idx_path(opts: &CliOpts, package: &str) -> PathBuf {
    let output_base = opts.output.strip_suffix('/').unwrap_or(&opts.output);

    PathBuf::from(format!("{output_base}/{package}/index.json.idx"))
}