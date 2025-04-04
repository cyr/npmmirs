use ahash::HashMap;
use compact_str::{CompactString, ToCompactString};
use tokio::io::AsyncReadExt;

use crate::metadata::package_index::PackageIndex;

#[derive(Default)]
pub struct MetaCache {
    pos_map: HashMap<CompactString, (usize, usize)>,
    data: Vec<u8>
}

impl MetaCache {
    pub async fn get(&self, buf: &mut Vec<u8>, package: &str) -> Option<PackageIndex> {
        let &(pos, len) = self.pos_map.get(package)?;

        let mut compressed = &self.data[pos..pos+len];

        let uncompressed_len: u64 = compressed.read_u64().await.unwrap();

        buf.resize(uncompressed_len as usize, 0u8);

        zstd::stream::copy_decode(&compressed[..], &mut buf[..]).expect("data is wrong :(");

        Some(bitcode::deserialize(&buf[..]).expect("bitcode deserialization failed from cache"))
    }

    pub fn insert(&mut self, package: &str, data: &[u8]) -> bool {
        if self.pos_map.contains_key(package) {
            return false
        }

        let len = data.len();
        let pos = self.data.len();

        self.data.extend_from_slice(data);

        self.pos_map.insert(package.to_compact_string(), (pos, len));

        true
    }
}