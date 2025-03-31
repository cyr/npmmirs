use std::sync::Arc;

use ahash::{HashMap, HashSet};
use compact_str::{CompactString, ToCompactString};
use nodejs_semver::{Range, Version};
use tokio::sync::RwLock;


#[derive(Default)]
pub struct PackageRangeCache {
    pub versions: Arc<RwLock<HashMap<CompactString, Ranges>>>,
    pub removed: Arc<RwLock<HashSet<CompactString>>>,
}

pub struct Ranges {
    pub inner: Vec<Range>,
}

impl Ranges {
    pub fn satisfies(&self, version: &Version) -> bool {
        for range in &self.inner {
            if range.satisfies(version) {
                return true
            }
        }

        false
    }

    pub fn max_satisfying<'a>(&self, versions: &'a [Version]) -> Vec<&'a Version> {
        let mut max_satisifying = Vec::new();

        for range in &self.inner {
            if let Some(s) = range.max_satisfying(versions) {
                max_satisifying.push(s);
            }
        }
        
        max_satisifying
    }
}

impl PackageRangeCache {
    pub async fn max_satisfying<'a>(&self, package: &str, versions: &'a [Version]) -> Vec<&'a Version> {
        {
            if self.removed.read().await.contains(package) {
                return Vec::new()
            }
        }

        let map = self.versions.read().await;

        if let Some(ranges) = map.get(package) {
            return ranges.max_satisfying(versions)
        }

        Vec::new()
    }

    pub async fn satisifies(&self, package: &str, version: &Version) -> bool {
        {
            if self.removed.read().await.contains(package) {
                return false
            }
        }

        let map = self.versions.read().await;

        if let Some(ranges) = map.get(package) {
            return ranges.satisfies(version)
        }

        false
    }

    pub async fn remove(&self, package: &str) {
        self.versions.write().await.remove(package);
        self.removed.write().await.insert(package.to_compact_string());
    }

    pub async fn insert(&self, package: &str, new_range: &Range) -> RangeCacheResult {
        {
            if self.removed.read().await.contains(package) {
                return RangeCacheResult { package_is_new: false, range_is_new: false }
            }
        }

        {
            if let Some(ranges) = self.versions.read().await.get(package) {
                for range in &ranges.inner {
                    if new_range.allows_all(range) {
                        return RangeCacheResult { package_is_new: false, range_is_new: false }
                    }
                }
            }
        }

        {
            let mut map = self.versions.write().await;

            let new_range = new_range.clone();

            if let Some(ranges) = map.get_mut(package) {
                ranges.inner.push(new_range);
                RangeCacheResult { package_is_new: false, range_is_new: true }
            } else {
                map.insert(package.to_compact_string(), Ranges { inner: vec![new_range] });
                RangeCacheResult { package_is_new: true, range_is_new: true }
            }
        }
    }
}

pub struct RangeCacheResult { pub package_is_new: bool, pub range_is_new: bool }