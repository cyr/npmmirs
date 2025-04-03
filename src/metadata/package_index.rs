use std::{collections::BTreeMap, path::Path};

use compact_str::{CompactString, ToCompactString};
use nodejs_semver::{Range, Version};
use serde::{Deserialize, Serialize};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, sync::RwLock};

use crate::{error::ErrorKind, meta_cache::MetaCache, CliOpts};

use super::{local_metadata_idx_path, sparse_metadata::{DepVersion, SparseMetadata, SubDep, VersionInfo}};

#[derive(Serialize, Deserialize, Debug)]
pub enum IdxDepVersion {
    Tag(CompactString),
    Range(Range),
    SubDep(SubDep),
    Other(CompactString),
}

impl From<DepVersion> for IdxDepVersion {
    fn from(value: DepVersion) -> Self {
        match value {
            DepVersion::Tag(s) => IdxDepVersion::Tag(s.into()),
            DepVersion::Range(range) => IdxDepVersion::Range(range),
            DepVersion::SubDep(sub_dep) => IdxDepVersion::SubDep(sub_dep),
            DepVersion::Other(other) => IdxDepVersion::Other(other.into()),
        }
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct IdxDep {
    pub package: CompactString,
    pub range: IdxDepVersion,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum TarballUrl {
    Short(CompactString),
    Full(String)
}

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct PackageIndex {
    pub dist_tags: BTreeMap<String, usize>,
    pub versions: Vec<Version>,
    pub tarballs: Vec<Option<TarballUrl>>,
    pub deps: Vec<Vec<IdxDep>>,
}

impl PackageIndex {
    pub fn from_sparse(opts: &CliOpts, value: SparseMetadata) -> Self {
        let Some(version_map) = value.versions else {
            return Default::default()
        };

        let mut versions = Vec::with_capacity(version_map.len());
        let mut tarballs = Vec::with_capacity(version_map.len());
        let mut deps = Vec::with_capacity(version_map.len());

        for (version, info) in version_map {
            versions.push(version);
            tarballs.push(info.as_ref().map(|v| strip_path(&v.dist.tarball, &value.name, &opts.registry_url)));
            
            let mut v_deps = Vec::with_capacity(
                info.as_ref().and_then(|v| v.dependencies.as_ref().map(|iv| iv.len())).unwrap_or(0) +
                info.as_ref().and_then(|v| v.dev_dependencies.as_ref().map(|iv| iv.len())).unwrap_or(0) +
                info.as_ref().and_then(|v| v.optional_dependencies.as_ref().map(|iv| iv.len())).unwrap_or(0) +
                info.as_ref().and_then(|v| v.peer_dependencies.as_ref().map(|iv| iv.len())).unwrap_or(0)
            );
            
            if let Some(v) = info {
                let VersionInfo {
                    dependencies,
                    dev_dependencies,
                    optional_dependencies,
                    peer_dependencies, .. 
                } = v;

                if let Some(deps) = dependencies {
                    v_deps.extend(deps.into_iter().map(|(package, range)| IdxDep { package: package.into(), range: range.into() }));
                }

                if !opts.no_dev_deps {
                    if let Some(deps) = dev_dependencies {
                        v_deps.extend(deps.into_iter().map(|(package, range)| IdxDep { package: package.into(), range: range.into() }));
                    }
                }

                if !opts.no_optional_deps {
                    if let Some(deps) = optional_dependencies {
                        v_deps.extend(deps.into_iter().map(|(package, range)| IdxDep { package: package.into(), range: range.into() }));
                    }
                }

                if !opts.no_peer_deps {
                    if let Some(deps) = peer_dependencies {
                        v_deps.extend(deps.into_iter().map(|(package, range)| IdxDep { package: package.into(), range: range.into() }));
                    }
                }
            }

            deps.push(v_deps);
        }

        let mut idx = Self {
            versions,
            tarballs,
            deps,
            ..Default::default()
        };
    
        if let Some(dist_tags) = value.dist_tags {
            idx.dist_tags = dist_tags.into_iter()
                .map(|(t, v)| (t, idx.pos_by_version(&v)))
                .filter(|(_, v)| v.is_some())
                .map(|(t, v)| (t, v.unwrap()))
                .collect();
        }

        idx
    }

    pub fn tarball_by_version(&self, version: &Version) -> Option<&TarballUrl>{
        let (pos, _) = self.versions.iter().enumerate()
            .find(|(_, v)| *v == version)?;

        self.tarball_by_pos(pos)
    }

    pub fn version_by_tag(&self, tag: &str) -> Option<&Version> {
        self.dist_tags.get(tag)
            .and_then(|pos| self.versions.get(*pos))
    }

    fn pos_by_version(&self, version: &Version) -> Option<usize> {
        self.versions.iter().enumerate()
            .find(|(_, v)| *v == version)
            .map(|(v, _)| v)
    }

    fn tarball_by_pos(&self, pos: usize) -> Option<&TarballUrl> {
        self.tarballs.get(pos)
            .and_then(|v| v.as_ref())
    }

    pub fn deps_by_version(&self, version: &Version) -> Option<&Vec<IdxDep>> {
        self.pos_by_version(version).and_then(|pos| self.deps.get(pos))
    }
}

fn strip_path(v: &str, package: &str, registry_url: &str) -> TarballUrl {
    v.strip_prefix(registry_url).and_then(|v| {
        v.find(package)
            .map(|pos| &v[package.len()+pos ..])
            .and_then(|iv| iv.strip_prefix("/-/")
                .map(|vv| TarballUrl::Short(vv.to_compact_string()))
            )
    }).unwrap_or_else(|| TarballUrl::Full(v.to_string()))
}

pub async fn write_package_idx(buf: &mut Vec<u8>, package: &str, target_path: &Path, pkg_idx: PackageIndex, meta_cache: &RwLock<MetaCache>) -> Result<(), ErrorKind> {
    let idx_path = target_path.parent().unwrap().join("index.json.idx");
    let idx_data = bitcode::serialize(&pkg_idx)?;

    let uncompressed_len = idx_data.len() as u64;
    let compressed = zstd::encode_all(&idx_data[..], 3)?;

    buf.clear();
    buf.write_u64(uncompressed_len).await?;
    buf.write_all(&compressed[..]).await?;

    meta_cache.write().await.insert(package, &buf[..]);

    let mut file = tokio::fs::File::create(&idx_path).await?;

    file.write_all(buf).await?;

    Ok(())
}

pub async fn read_package_idx(opts: &CliOpts, buf: &mut Vec<u8>, package: &str) -> Result<usize, ErrorKind> {
    let mut idx_file = tokio::fs::File::open(&local_metadata_idx_path(opts, package)).await?;

    let idx_len = idx_file.metadata().await?.len() as usize;
    
    buf.reserve_exact(idx_len);

    idx_file.read_to_end(buf).await.map_err(ErrorKind::from)
}