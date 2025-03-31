use std::{collections::BTreeMap, str::FromStr};

use compact_str::CompactString;
use nodejs_semver::{Range, Version};
use serde::{de::Visitor, Deserialize, Serialize};

struct VersionRangeDeserializer;

impl<'de> Visitor<'de> for VersionRangeDeserializer {
    type Value = DepVersion;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a tag or npm version range")
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
        where
            E: serde::de::Error, {
        
        match Range::from_str(value) {
            Ok(r) => Ok(DepVersion::Range(r)),
            Err(_) => {
                if value.trim().is_empty()
                    || value.starts_with("link:")
                    || value.starts_with("git")
                    || value.starts_with("gist")
                    || value.starts_with("workspace:")
                    || value.starts_with("http")
                    || value.starts_with("file:")
                    || value.starts_with(".") 
                    || value.starts_with("/") { 
                    return Ok(DepVersion::Other(value.to_string()))
                }

                if let Some(sub_package) = value.strip_prefix("npm:") {
                    if let Some((sub_pkg_name, Ok(sub_pkg_v))) = sub_package
                        .split_once('@')
                        .map(|(dep, v)| (dep, Range::from_str(v))) {
                        let sub_dep = SubDep {
                            package: sub_pkg_name.into(),
                            range: sub_pkg_v,
                        };

                        return Ok(DepVersion::SubDep(sub_dep))
                    }

                }

                Ok(DepVersion::Tag(value.to_string()))
            }
        }
    }
}

#[derive(Serialize, Debug)]
pub enum DepVersion {
    Tag(String),
    Range(Range),
    SubDep(SubDep),
    Other(String),
}

impl<'de> Deserialize<'de> for DepVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de> {
        deserializer.deserialize_string(VersionRangeDeserializer)
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct Dep {
    pub package: String,
    pub range: DepVersion,
}

#[derive(Deserialize, Serialize, Debug)]
pub struct SubDep {
    pub package: CompactString,
    pub range: Range,
}

#[derive(Deserialize, Debug)]
pub struct SparseMetadata {
    pub name: String,
    #[serde(rename = "dist-tags")]
    pub dist_tags: Option<BTreeMap<String, Version>>,
    pub versions: Option<BTreeMap<Version, Option<VersionInfo>>>,
}

#[derive(Deserialize, Debug, Default)]
pub struct VersionInfo {
    pub dist: Dist,
    pub dependencies: Option<BTreeMap<String, DepVersion>>,
    #[serde(rename = "devDependencies")]
    pub dev_dependencies: Option<BTreeMap<String, DepVersion>>,
    #[serde(rename = "optionalDependencies")]
    pub optional_dependencies: Option<BTreeMap<String, DepVersion>>,
    #[serde(rename = "peerDependencies")]
    pub peer_dependencies: Option<BTreeMap<String, DepVersion>>,
}

#[derive(Deserialize, Debug, Default)]
pub struct Dist {
    pub tarball: String,
}

