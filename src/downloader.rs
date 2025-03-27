
use std::path::PathBuf;
use std::{path::Path, sync::Arc};

use async_channel::{bounded, Sender, Receiver};
use reqwest::header::CONTENT_LENGTH;
use reqwest::{Client, StatusCode};
use tokio::{task::JoinHandle, io::AsyncWriteExt};

use crate::checksum::Checksum;
use crate::metadata::local_metadata_path;
use crate::progress::Progress;
use crate::error::{ErrorKind, Result};
use crate::CliOpts;

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
    pub fn build(num_threads: u8) -> Self {
        let (sender, receiver) = bounded(1024);

        let mut tasks = Vec::with_capacity(num_threads as usize);
        let progress = Progress::new();
        let http_client = reqwest::Client::new();

        for _ in 0..num_threads {
            let task_receiver: Receiver<Download> = receiver.clone();
            let task_progress = progress.clone();
            let task_http_client = http_client.clone();

            let handle = tokio::spawn(async move {
                while let Ok(dl) = task_receiver.recv().await {
                    _ = Downloader::download_and_track(&task_http_client, task_progress.clone(), dl).await;
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

    async fn download_and_track(http_client: &Client, progress: Progress, dl: Download) -> Result<()> {
        match download_file(progress.clone(), http_client, dl, 
            |downloaded| progress.bytes.inc_success(downloaded)
        ).await {
            Ok(true) => progress.files.inc_success(1),
            Ok(false) => progress.files.inc_skipped(1),
            Err(e) => {
                match e {
                    ErrorKind::Checksum { .. } => progress.files.inc_failed(1),
                    ErrorKind::Download { .. } => {
                        progress.files.inc_skipped(1);
                    },
                    _ => progress.files.inc_skipped(1),
                }
            }
        }

        Ok(())
    }

    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }
}

async fn download_file<F>(progress: Progress, http_client: &Client, download: Download, mut progress_cb: F) -> Result<bool>
    where F: FnMut(u64) {
    
    let mut downloaded = false;

    if !download.target_path.exists() {
        create_dirs(&download.target_path).await?;

        let mut output = tokio::fs::File::create(&download.target_path).await?;

        let mut response = http_client.get(download.url.as_str()).send().await?;

        if response.status() == StatusCode::NOT_FOUND {
            drop(output);
            tokio::fs::remove_file(&download.target_path).await?;
            return Err(ErrorKind::Download { url: download.url.clone(), status_code: response.status() })
        }

        if let Some(content_len) = response.headers().get(CONTENT_LENGTH) {
            let size: u64 = content_len.to_str().expect("junk in content length").parse::<u64>()?;
            progress.bytes.inc_total(size);
        }

        if let Some(expected_checksum) = download.checksum {
            let mut hasher = expected_checksum.create_hasher();
    
            while let Some(chunk) = response.chunk().await? {
                output.write_all(&chunk).await?;
                hasher.consume(&chunk);
        
                progress_cb(chunk.len() as u64);
            }
    
            let checksum = hasher.compute();
    
            if expected_checksum != checksum {
                drop(output);
                tokio::fs::remove_file(&download.target_path).await?;
                return Err(ErrorKind::Checksum { 
                    url: download.url, 
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

pub async fn create_dirs<P: AsRef<Path>>(path: P) -> Result<()> {
    if let Some(parent_dir) = path.as_ref().parent() {
        if !parent_dir.exists() {
            tokio::fs::create_dir_all(parent_dir).await?;
        }
    }

    Ok(())
}

#[derive(Debug)]
pub struct Download {
    pub url: String,
    pub checksum: Option<Checksum>,
    pub target_path: PathBuf,
}

impl Download {
    pub fn metadata(opts: &CliOpts, package: &str) -> Download {
        let url_base = opts.registry_url.strip_suffix('/').unwrap_or(&opts.registry_url);

        Download {
            url: format!("{}/{}", url_base, package),
            checksum: None,
            target_path: local_metadata_path(opts, &package)
        }
    }

    pub fn versioned_package(opts: &CliOpts, package: &str, url: String) -> Download {
        let output_base = opts.output.strip_suffix('/').unwrap_or(&opts.output);

        let target_path = if let Some(last_part) = url.strip_prefix(&opts.registry_url) {
            let path_part = last_part.strip_prefix('/').unwrap_or(last_part);
            format!("{output_base}/{path_part}")
        } else {
            let name = url.split('/').last().unwrap();
            format!("{output_base}/{package}/-/{name}")
        };

        Download {
            url,
            checksum: None,
            target_path: PathBuf::from(target_path)
        }
    }
}