use std::{fmt::Display, mem, str::FromStr, sync::Arc};

use ahash::{HashMap, HashSet, HashSetExt};
use indicatif::HumanBytes;
use nodejs_semver::{Range, Version};
use serde_json::{Map, Value};
use tokio::{fs::read_to_string, sync::RwLock};
use walkdir::WalkDir;

use crate::{downloader::{Download, Downloader}, error::{ErrorKind, NpmError}, log, metadata::{local_metadata_path, manifest::Manifest}, CliOpts};

pub struct MirrorResult {
    new_packages: u64,
    new_packages_bytes: u64,
}

impl Display for MirrorResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{} new packages ({})", self.new_packages, HumanBytes(self.new_packages_bytes)))
    }
}

pub async fn mirror(opts: &CliOpts, downloader: Downloader) -> Result<MirrorResult, NpmError> {
    let range_cache = PackageRangeCache::default();

    downloader.progress().set_total_steps(3);
    downloader.progress().next_step("Getting metadata").await;

    download_metadata(opts, &downloader, &range_cache).await
        .map_err(NpmError::DownloadingDependencies)?;

    downloader.progress().next_step("Getting child metadata").await;

    download_child_metadata(opts, &downloader, &range_cache).await
        .map_err(NpmError::DownloadingChildDependencies)?;
    
    downloader.progress().next_step("Downloading packages").await;

    let result = download_packages(opts, &downloader, &range_cache).await
        .map_err(NpmError::DownloadingPackages)?;

    // TODO: add step to remove non-existing versions from index.json-files

    Ok(result)
}

async fn download_packages(opts: &CliOpts, downloader: &Downloader, range_cache: &PackageRangeCache) -> Result<MirrorResult, ErrorKind> {
    let mut progress_bar = downloader.progress().create_download_progress_bar().await;

    let map = range_cache.versions.read().await;

    for (package, ranges) in map.iter() {
        let pkg_o = match get_package_metadata(opts, package).await {
            Ok(v) => v,
            Err(e) => {
                log(format!("will not fetch packages for {package}: {e}"));
                continue
            },
        };

        let Some(Some(versions)) = pkg_o.get("versions").map(Value::as_object) else {
            log(format!("{package} does not have a versions object"));
            continue
        };

        for version_key in versions.keys() {
            let version = match Version::from_str(&version_key) {
                Ok(v) => v,
                Err(e) => {
                    log(format!("invalid version in {package}: {e}"));
                    continue
                }
            };

            if !ranges.satisfies(&version) {
                continue
            }

            let Some(Some(version_o)) = versions.get(version_key).map(Value::as_object) else {
                log(format!("{package}:{version_key} does not exist"));
                continue
            };

            let Some(Some(dist)) = version_o.get("dist").map(Value::as_object) else {
                log(format!("{package}:{version_key} does not have a dist object"));
                continue
            };

            let Some(Some(url)) = dist.get("tarball").map(Value::as_str) else {
                log(format!("{package}:{version_key} does not have a tarball field"));
                continue
            };

            downloader.queue(
                Download::versioned_package(opts, package, url.to_string())
            ).await?;

            downloader.progress().update_for_files(&mut progress_bar);
        }
    }

    downloader.progress().wait_for_completion(&mut progress_bar).await;

    Ok(MirrorResult {
        new_packages: downloader.progress().files.success(),
        new_packages_bytes: downloader.progress().bytes.success(),
    })
}

async fn download_child_metadata(opts: &CliOpts, downloader: &Downloader, range_cache: &PackageRangeCache) -> Result<(), ErrorKind> {
    let mut progress_bar = downloader.progress().create_download_progress_bar().await;

    let mut visited = HashSet::<(String, String)>::new();

    let mut packages: Vec<String> = range_cache.versions.read().await
        .keys()
        .map(|v| v.clone())
        .collect();

    let mut new_packages = Vec::new();

    while let Some(package) = packages.pop() {
        let path = local_metadata_path(opts, &package);

        let Ok(s) = tokio::fs::read_to_string(&path).await else {
            log(format!("unable to read {}", path.to_string_lossy()));
            continue
        };

        let o: Value = serde_json::from_str(&s)?;

        let Some(Some(versions)) = o.get("versions").map(Value::as_object) else {
            log(format!("{} does not have a versions object", path.to_string_lossy()));
            continue
        };

        for version_key in versions.keys() {
            if !visited.insert((package.clone(), version_key.clone())) {
                continue
            }

            let version = match Version::from_str(&version_key) {
                Ok(v) => v,
                Err(e) => {
                    log(format!("invalid version in {package}: {e}"));
                    continue
                }
            };

            if !range_cache.satisifies(&package, &version).await {
                continue
            }

            let Some(Some(version_o)) = versions.get(version_key).map(Value::as_object) else {
                log(format!("{}: version {version_key} is not an object", path.to_string_lossy()));
                continue
            };

            if let Some(Some(deps)) = version_o.get("dependencies").map(Value::as_object) {
                populate_child_deps(&package, opts, deps, downloader, range_cache, &mut new_packages).await?;
            };

            if let Some(Some(deps)) = version_o.get("devDependencies").map(Value::as_object) {
                populate_child_deps(&package, opts, deps, downloader, range_cache, &mut new_packages).await?;
            };
        }

        if packages.is_empty() {
            mem::swap(&mut packages, &mut new_packages);
            downloader.progress().wait_for_idle(&mut progress_bar).await;
        }
    }

    progress_bar.finish_using_style();
    
    Ok(())
}

async fn populate_child_deps(package: &str, opts: &CliOpts, deps_o: &Map<String, Value>, downloader: &Downloader, range_cache: &PackageRangeCache, packages: &mut Vec<String>) -> Result<(), ErrorKind> {
    for dep in deps_o.keys() {
        if let Some(Some(range)) = deps_o.get(dep).map(Value::as_str) {
            if range.trim().is_empty() {
                continue
            }

            if range.starts_with("link:") {
                continue
            }

            if range.starts_with("git") {
                continue
            }

            if range.starts_with("gist") {
                continue
            }

            if range.starts_with("workspace") {
                continue
            }

            if range.starts_with("http") {
                continue
            }

            if range.starts_with("file") {
                continue
            }

            if range.starts_with(".") {
                continue
            }

            if let Some(sub_package) = range.strip_prefix("npm:") {
                // add subpackage to list here?
                continue
            }

            let package_range = match Range::from_str(range) {
                Ok(v) => v,
                Err(e) => {
                    log(format!("invalid dependency range {range} for {dep} in {package}: {e}"));
                    continue
                }
            };

            let res = range_cache.insert(dep, &package_range).await;
            
            if res.package_is_new {
                downloader.queue(Download::metadata(opts, dep)).await?;
            }
            
            if res.package_is_new || res.range_is_new {
                if !packages.contains(dep) {
                    packages.push(dep.clone());
                } 
            }
        }
    }

    Ok(())
}

async fn download_metadata(opts: &CliOpts, downloader: &Downloader, range_cache: &PackageRangeCache) -> Result<(), ErrorKind> {
    let mut dl_progress_bar = downloader.progress().create_download_progress_bar().await;

    for entry in WalkDir::new(&opts.manifests_path) {
        let entry = entry?;

        if entry.file_type().is_dir() {
            continue
        }

        let d = read_to_string(entry.path()).await?;

        let manifest: Manifest = serde_json::from_str(&d)?;

        for (package, version_range) in manifest.dependencies {
            let res = range_cache.insert(&package, &version_range).await;

            if res.package_is_new {
                downloader.queue(Download::metadata(&opts, &package)).await?;
            }
        }
    }

    downloader.progress().wait_for_completion(&mut dl_progress_bar).await;

    Ok(())
}

async fn get_package_metadata(opts: &CliOpts, package: &str) -> Result<Value, ErrorKind> {
    
    let path = local_metadata_path(opts, &package);

    let s = tokio::fs::read_to_string(&path).await?;

    serde_json::from_str(&s).map_err(ErrorKind::from)
}

#[derive(Default)]
pub struct PackageRangeCache {
    pub versions: Arc<RwLock<HashMap<String, Ranges>>>,
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
}

impl PackageRangeCache {
    pub async fn satisifies(&self, package: &str, version: &Version) -> bool {
        let map = self.versions.read().await;

        if let Some(ranges) = map.get(package) {
            return ranges.satisfies(version)
        }

        false
    }

    pub async fn insert(&self, package: &str, new_range: &Range) -> RangeCacheResult {
        {
            let map = self.versions.read().await;

            if let Some(ranges) = map.get(package) {
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
                map.insert(package.to_owned(), Ranges { inner: vec![new_range] });
                RangeCacheResult { package_is_new: true, range_is_new: true }
            }
        }
    }
}

pub struct RangeCacheResult { pub package_is_new: bool, pub range_is_new: bool }