use reqwest::StatusCode;
use thiserror::Error;

use crate::downloader::Download;


pub type Result<T> = std::result::Result<T, ErrorKind>;

#[derive(Error, Debug)]
pub enum NpmError {
    #[error("downloading main dependencies failed: {}", .0)]
    Dependencies(ErrorKind),

    #[error("downloading child dependencies failed: {}", .0)]
    ChildDependencies(ErrorKind),
    
    #[error("downloading packages failed: {}", .0)]
    Packages(ErrorKind),

}

#[derive(Error, Debug)]
pub enum ErrorKind {
    #[error("async channel send error: {}", .0)]
    AsyncChannelSend(#[from]async_channel::SendError<Download>),

    #[error("failed downloading {}: {status_code}", .url)]
    Download { url: String, status_code: StatusCode },

    #[error("")]
    Checksum { url: String, expected: String, hash: String },

    #[error("failed to parse as checksum: {value}")]
    IntoChecksum { value: String },

    #[error("failed to parse hex: {}", .0)]
    FromHex(#[from]hex::FromHexError),

    #[error("io error: {}", .0)]
    Io(#[from]std::io::Error),

    #[error("reqwest error: {}", .0)]
    Reqwest(#[from]reqwest::Error),

    #[error("unable to parse string as integer: {}", .0)]
    ParseInt(#[from]std::num::ParseIntError),

    #[error("unable to probe manifest path: {}", .0)]
    Walkdir(#[from]walkdir::Error),

    #[error("json serialization failed: {}", .0)]
    Serde(#[from]serde_json::Error),

    #[error("bitcode serialization failed: {}", .0)]
    Bitcode(#[from]bitcode::Error),

    #[error("semver parse error: {}", .0)]
    SemVer(#[from]nodejs_semver::SemverError),
}