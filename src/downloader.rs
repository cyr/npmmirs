
use std::path::PathBuf;
use std::{path::Path, sync::Arc};

use async_channel::{bounded, Sender, Receiver};
use reqwest::{header::CONTENT_LENGTH, Client, StatusCode};
use tokio::{fs::File, io::{AsyncWriteExt, BufWriter}, task::JoinHandle};

use crate::checksum::Checksum;
use crate::metadata::{local_metadata_path, package_index::{write_package_idx, PackageIndex, TarballUrl}, sparse_metadata::SparseMetadata};
use crate::progress::Progress;
use crate::error::{ErrorKind, Result};
use crate::{log, CliOpts};

pub struct DownloadTask {
    opts: Arc<CliOpts>,
    receiver: Receiver<Download>,
    progress: Progress,
    http_client: Client,
}

impl DownloadTask {
    async fn download_and_track(&self, buf: &mut Vec<u8>, dl: Download) -> Result<()> {
        match self.download(buf, dl, |b| self.progress.bytes.inc_success(b)).await {
            Ok(true) => self.progress.files.inc_success(1),
            Ok(false) => self.progress.files.inc_skipped(1),
            Err(e) => {
                match e {
                    ErrorKind::Download { url: _, status_code: StatusCode::NOT_FOUND } => {
                        self.progress.files.inc_skipped(1);
                    },
                    ErrorKind::Checksum { .. } |
                    ErrorKind::Download { .. } => self.progress.files.inc_failed(1),
                    _ => self.progress.files.inc_skipped(1),
                }
            }
        }

        Ok(())
    }

    async fn download<F>(&self, buf: &mut Vec<u8>, dl: Download, progress_cb: F) -> Result<bool> where F: FnMut(u64) { 
        match dl {
            Download::Metadata { url, target_path } =>
                self.download_metadata(buf, url, target_path, progress_cb).await,
            Download::Tarball { url, target_path, checksum } =>
                self.download_tarball(url, target_path, checksum, progress_cb).await,
        }
    }

    async fn download_metadata<F>(&self, buf: &mut Vec<u8>, url: String, target_path: PathBuf, mut progress_cb: F) -> Result<bool> where F: FnMut(u64) { 
        let mut downloaded = false;

        if !target_path.exists() {
            let mut response = self.http_client.get(url.as_str()).send().await?;

            if !response.status().is_success() {
                return Err(ErrorKind::Download { url, status_code: response.status() })
            }

            create_dirs(&target_path).await?;

            if let Some(content_len) = response.headers().get(CONTENT_LENGTH) {
                let size: u64 = content_len.to_str().expect("junk in content length").parse::<u64>()?;
                self.progress.bytes.inc_total(size);
                buf.reserve(size as usize);
            }

            while let Some(chunk) = response.chunk().await? {
                AsyncWriteExt::write_all(buf, &chunk).await?;
        
                progress_cb(chunk.len() as u64);
            }

            let sparse_metadata: SparseMetadata = match serde_json::from_slice(buf) {
                Ok(v) => v,
                Err(e) => {
                    log(format!("unable to parse sparse version of package metadata {url}: {e}"));
                    return Err(e.into())
                },
            };

            tokio::fs::write(&target_path, buf).await?;

            let pkg_idx = PackageIndex::from_sparse(&self.opts, sparse_metadata);
            let idx_path = target_path.parent().unwrap().join("index.json.idx");

            write_package_idx(&idx_path, pkg_idx).await?;

            downloaded = true;
        }

        Ok(downloaded)
    }

    async fn download_tarball<F>(&self, url: String, target_path: PathBuf, checksum: Option<Checksum>, mut progress_cb: F) -> Result<bool> where F: FnMut(u64) {
        let mut downloaded = false;

        if !target_path.exists() {
            let mut response = self.http_client.get(url.as_str()).send().await?;

            if !response.status().is_success() {
                return Err(ErrorKind::Download { url, status_code: response.status() })
            }

            create_dirs(&target_path).await?;

            let mut output = BufWriter::new(File::create(&target_path).await?);

            if let Some(content_len) = response.headers().get(CONTENT_LENGTH) {
                let size: u64 = content_len.to_str().expect("junk in content length").parse::<u64>()?;
                self.progress.bytes.inc_total(size);
            }

            if let Some(expected_checksum) = checksum {
                let mut hasher = expected_checksum.create_hasher();

                while let Some(chunk) = response.chunk().await? {
                    output.write_all(&chunk).await?;
                    hasher.consume(&chunk);
            
                    progress_cb(chunk.len() as u64);
                }

                let checksum = hasher.compute();

                if expected_checksum != checksum {
                    drop(output);
                    tokio::fs::remove_file(&target_path).await?;
                    return Err(ErrorKind::Checksum { 
                        url, 
                        expected: expected_checksum.to_string(), 
                        hash: checksum.to_string() 
                    })
                }
            } else {
                while let Some(chunk) = response.chunk().await? {
                    output.write_all(&chunk).await?;
            
                    progress_cb(chunk.len() as u64);
                }
            }

            output.flush().await?;
            downloaded = true;
        }

        Ok(downloaded)
    }
}

#[derive(Clone)]
pub struct Downloader {
    sender: Sender<Download>,
    _tasks: Arc<Vec<JoinHandle<()>>>,
    progress: Progress
}

impl Default for Downloader {
    fn default() -> Self {
        let (sender, _) = bounded(1);
        Self {
            sender,
            _tasks: Default::default(),
            progress: Default::default()
        }
    }
}

impl Downloader {
    pub fn build(opts: &CliOpts) -> Self {
        let (sender, receiver) = bounded(1024);

        let mut tasks = Vec::with_capacity(opts.dl_threads as usize);
        let progress = Progress::new();
        let http_client = reqwest::Client::new();

        let task_opts = Arc::new(opts.to_owned());

        for _ in 0..opts.dl_threads {
            let dl_task = DownloadTask {
                opts: task_opts.clone(),
                receiver: receiver.clone(),
                progress: progress.clone(),
                http_client: http_client.clone(),
            };

            let mut buf = Vec::with_capacity(1024*1024);

            let handle = tokio::spawn(async move {
                while let Ok(dl) = dl_task.receiver.recv().await {
                    buf.clear();
                    _ = dl_task.download_and_track(&mut buf, dl).await;
                }
            });

            tasks.push(handle);
        }

        Self {
            sender,
            _tasks: Arc::new(tasks),
            progress
        }
    }

    pub async fn queue(&self, download_entry: Download) -> Result<()> {
        self.progress.files.inc_total(1);

        self.sender.send(download_entry).await?;

        Ok(())
    }

    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }
}

pub async fn create_dirs<P: AsRef<Path>>(path: P) -> Result<()> {
    if let Some(parent_dir) = path.as_ref().parent() {
        if !parent_dir.exists() {
            tokio::fs::create_dir_all(parent_dir).await?;
        }
    }

    Ok(())
}

pub enum Download {
    Metadata {
        url: String,
        target_path: PathBuf,
    },
    Tarball {
        url: String,
        target_path: PathBuf,
        checksum: Option<Checksum>,
    }
}

impl Download {
    pub fn metadata(opts: &CliOpts, package: &str) -> Download {
        let url_base = opts.registry_url.strip_suffix('/').unwrap_or(&opts.registry_url);

        Download::Metadata {
            url: format!("{}/{}", url_base, package),
            target_path: local_metadata_path(opts, package)
        }
    }

    pub fn tarball(opts: &CliOpts, package: &str, url: &TarballUrl) -> Download {
        let output_base = opts.output.strip_suffix('/').unwrap_or(&opts.output);
        let url_base = opts.registry_url.strip_prefix('/').unwrap_or(&opts.registry_url);

        let url = match url {
            TarballUrl::Short(short) => {
                format!("{url_base}/{package}/-/{short}")
            },
            TarballUrl::Full(v) => v.to_string(),
        };

        let target_path = if let Some(last_part) = url.strip_prefix(opts.registry_url.as_str()) {
            let path_part = last_part.strip_prefix('/').unwrap_or(last_part);
            format!("{output_base}/{path_part}")
        } else {
            let name = url.split('/').last().unwrap();
            format!("{output_base}/{package}/-/{name}")
        };

        Download::Tarball {
            url,
            checksum: None,
            target_path: PathBuf::from(target_path)
        }
    }
}