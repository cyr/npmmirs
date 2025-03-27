use std::fmt::Display;

use sha2::{digest::{FixedOutput, Update}, Digest, Sha512};

use crate::error::ErrorKind;

#[derive(Debug, PartialEq)]
pub enum Checksum {
    Sha512([u8; 64])
}

impl TryFrom<&str> for Checksum {
    type Error = ErrorKind;

    fn try_from(value: &str) -> std::prelude::v1::Result<Self, Self::Error> {
        match value.len() {
            128 => {
                let mut bytes = [0_u8; 64];
                hex::decode_to_slice(value, &mut bytes)?;
                Ok(bytes.into())
            }
            _ => Err(ErrorKind::IntoChecksum { value: value.to_string() })
        }
    }
}

impl From<[u8; 64]> for Checksum {
    fn from(value: [u8; 64]) -> Self {
        Self::Sha512(value)
    }
}

impl Display for Checksum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Checksum::Sha512(v) => f.write_str(&hex::encode(v)),
        }
    }
}

impl Checksum {
    pub fn create_hasher(&self) -> Box<dyn Hasher> {
        match self {
            Checksum::Sha512(_) => Box::new(Sha512Hasher::new()),
        }
    }
}

pub trait Hasher : Sync + Send {
    fn consume(&mut self, data: &[u8]);
    fn compute(self: Box<Self>) -> Checksum;
}

pub struct Sha512Hasher {
    hasher: Sha512
}

impl Sha512Hasher {
    pub fn new() -> Self {
        Self {
            hasher: sha2::Sha512::new()
        }
    }
}

impl Hasher for Sha512Hasher {
    fn consume(&mut self, data: &[u8]) {
        Update::update(&mut self.hasher, data)
    }

    fn compute(self: Box<Self>) -> Checksum {
        Checksum::Sha512(self.hasher.finalize_fixed().into())
    }
}