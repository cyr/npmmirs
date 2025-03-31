use std::{fmt::Display, mem, time::Duration};

use ahash::{HashSet, HashSetExt};
use compact_str::{CompactString, ToCompactString};
use indicatif::{HumanBytes, MultiProgress, ProgressBar};
use nodejs_semver::{Range, Version};
use tokio::{fs::read_to_string, task::JoinHandle, time::sleep};
use walkdir::WalkDir;

use crate::{downloader::{Download, Downloader}, error::{ErrorKind, NpmError}, log, metadata::{manifest::Manifest, package_index::{read_package_idx, IdxDep, IdxDepVersion, PackageIndex}}, progress::Progress, range_cache::PackageRangeCache, CliOpts};

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

    let mut buf: Vec<u8> = vec![0u8; 1024*8];

    downloader.progress().set_total_steps(3);
    downloader.progress().next_step("Direct deps").await;

    download_metadata(opts, &downloader, &range_cache).await
        .map_err(NpmError::Dependencies)?;

    downloader.progress().next_step("Descendants").await;

    download_child_metadata(&mut buf, opts, &downloader, &range_cache).await
        .map_err(NpmError::ChildDependencies)?;
    
    downloader.progress().next_step("Packages").await;

    let result = download_packages(&mut buf, opts, &downloader, &range_cache).await
        .map_err(NpmError::Packages)?;

    // TODO: add step to remove non-existing versions from index.json-files

    Ok(result)
}

async fn download_packages(buf: &mut Vec<u8>, opts: &CliOpts, downloader: &Downloader, range_cache: &PackageRangeCache) -> Result<MirrorResult, ErrorKind> {
    let multi_bar = MultiProgress::new();

    let proc_progress = Progress::with_step("Processing");

    let proc_pb = multi_bar.add(proc_progress.create_processing_progress_bar().await);
    let dl_pb = multi_bar.add(downloader.progress().create_download_progress_bar().await);
    
    let _updater = spawn_updater(vec![
        (proc_progress.clone(), proc_pb.clone()),
        (downloader.progress(), dl_pb.clone())
    ]).await;

    let map = range_cache.versions.read().await;

    proc_progress.files.inc_total(map.len() as u64);

    for (package, ranges) in map.iter() {
        proc_progress.update_for_files(&proc_pb);
        buf.clear();
        let idx = match read_package_idx(opts, buf, package).await {
            Ok(v) => v,
            Err(e) => {
                // we don't particularly care if we can't read the idx file. probably just means that there is no such
                // package and the metadata was not downloaded. happens plenty of times during greedy runs, no biggie.
                if opts.verbose {
                    log(format!("unable to read idx for {package}: {e}"));
                }

                proc_progress.files.inc_failed(1);
                
                continue
            }
        };
        
        if opts.greedy {
            for version in idx.versions.iter().filter(|v| ranges.satisfies(v)) {
                let Some(tarball_url) = idx.tarball_by_version(version) else {
                    continue
                };
    
                downloader.queue(Download::tarball(opts, package, tarball_url)).await?;
            }
        } else {
            for version in ranges.max_satisfying(&idx.versions) {
                let Some(tarball_url) = idx.tarball_by_version(version) else {
                    continue
                };
    
                downloader.queue(Download::tarball(opts, package, tarball_url)).await?;
            }
        }

        proc_progress.files.inc_success(1);
        
    }
    
    downloader.progress().wait_for_completion(&dl_pb).await;

    Ok(MirrorResult {
        new_packages: downloader.progress().files.success(),
        new_packages_bytes: downloader.progress().bytes.success(),
    })
}

async fn spawn_updater(progress_pairs: Vec<(Progress, ProgressBar)>) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            for (progress, pb) in &progress_pairs {
                progress.update_for_files(pb);
            }
    
            sleep(Duration::from_millis(100)).await
        }
    })
}

async fn download_child_metadata(buf: &mut Vec<u8>, opts: &CliOpts, downloader: &Downloader, range_cache: &PackageRangeCache) -> Result<(), ErrorKind> {
    let proc_progress = Progress::with_step("Resolving");

    let multibar = MultiProgress::new();
    let proc_pb = multibar.add(proc_progress.create_processing_progress_bar().await);
    let dl_pb = multibar.add(downloader.progress().create_download_progress_bar().await);

    let _updater = spawn_updater(vec![
        (proc_progress.clone(), proc_pb.clone()),
        (downloader.progress(), dl_pb.clone())
    ]).await;

    let mut visited = HashSet::<(CompactString, Version)>::new();

    let mut packages: Vec<CompactString> = range_cache.versions.read().await
        .keys()
        .cloned()
        .collect();

    proc_progress.files.inc_total(packages.len() as u64);

    let mut new_packages = Vec::new();

    while let Some(package) = packages.pop() {
        proc_progress.files.inc_success(1);

        buf.clear();
        let idx = match read_package_idx(opts, buf, &package).await {
            Ok(v) => v,
            Err(e) => {
                if opts.verbose {
                    log(format!("unable to read idx for {package}: {e}"));
                }

                range_cache.remove(&package).await;

                continue
            }
        };

        if opts.greedy {
            for version in &idx.versions {
                let pkg_v = (package.clone(), version.clone());
                if visited.contains(&pkg_v) {
                    continue 
                }
    
                visited.insert(pkg_v);
    
                if !range_cache.satisifies(&package, version).await {
                    continue
                }
    
                if let Some(deps) = idx.deps_by_version(version) {
                    populate_child_deps(&idx, opts, deps, downloader, range_cache, &mut new_packages).await?;
                }
            }
        } else {
            for version in range_cache.max_satisfying(&package, &idx.versions).await {
                let pkg_v = (package.clone(), version.clone());
                if visited.contains(&pkg_v) {
                    continue 
                }
    
                visited.insert(pkg_v);

                if let Some(deps) = idx.deps_by_version(version) {
                    populate_child_deps(&idx, opts, deps, downloader, range_cache, &mut new_packages).await?;
                }
            }
        }

        if packages.is_empty() {
            mem::swap(&mut packages, &mut new_packages);
            downloader.progress().wait_for_idle(&dl_pb).await;

            proc_progress.files.reset();
            proc_progress.files.inc_total(packages.len() as u64);
        }
    }

    dl_pb.finish_using_style();
    
    Ok(())
}

async fn populate_child_deps(idx: &PackageIndex, opts: &CliOpts, deps: &Vec<IdxDep>, downloader: &Downloader, range_cache: &PackageRangeCache, packages: &mut Vec<CompactString>) -> Result<(), ErrorKind> {
    for dep in deps {
        match &dep.range {
            IdxDepVersion::Tag(tag) => {
                if let Some(version) = idx.version_by_tag(tag.as_str()) {
                    let range = Range::parse(version.to_compact_string())?;
                    process_version(opts, downloader, range_cache, &dep.package, &range, packages).await?;
                }
            },
            IdxDepVersion::Range(range) => {
                process_version(opts, downloader, range_cache, &dep.package, range, packages).await?;
            },
            IdxDepVersion::SubDep(sub_dep) => {
                process_version(opts, downloader, range_cache, &sub_dep.package, &sub_dep.range, packages).await?;
            },
            _ => (),
        }
    }

    Ok(())
}

async fn process_version(opts: &CliOpts, downloader: &Downloader, range_cache: &PackageRangeCache, dep: &str, version_range: &Range, packages: &mut Vec<CompactString>) -> Result<(), ErrorKind> {
    let res = range_cache.insert(dep, version_range).await;
            
    if res.package_is_new {
        downloader.queue(Download::metadata(opts, dep)).await?;
    }
    
    if (res.package_is_new || res.range_is_new) && !packages.iter().any(|v| v == dep) {
        packages.push(dep.to_compact_string());
    }

    Ok(())
}

async fn download_metadata(opts: &CliOpts, downloader: &Downloader, range_cache: &PackageRangeCache) -> Result<(), ErrorKind> {
    let mut dl_pb = downloader.progress().create_download_progress_bar().await;

    let _updater = spawn_updater(vec![
        (downloader.progress(), dl_pb.clone())
    ]).await;

    _updater.abort();

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
                downloader.queue(Download::metadata(opts, &package)).await?;
            }
        }
    }

    downloader.progress().wait_for_completion(&mut dl_pb).await;

    Ok(())
}
