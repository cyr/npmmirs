
use std::{fmt::Display, process::exit, sync::Arc};

use clap::{command, arg, Parser};
use downloader::Downloader;
use meta_cache::MetaCache;
use mirror::mirror;
use tokio::sync::RwLock;

mod downloader;
mod error;
mod progress;
mod checksum;
mod mirror;
mod metadata;
mod range_cache;
mod meta_cache;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    let opts = CliOpts::parse();

    let meta_cache = Arc::new(RwLock::new(MetaCache::default()));
    let downloader = Downloader::build(&opts, meta_cache.clone());

    log("Mirroring started");
    match mirror(&opts, downloader, &meta_cache).await {
        Ok(res) => {
            log(format!("Mirroring completed: {res}"));
            exit(0)
        },
        Err(e) => {
            log(format!("Mirroring failed: {e}"));
            exit(-1)
        }
    }
}

#[derive(Parser, Clone)]
#[command(author, version, about)]
struct CliOpts {
    #[arg(short, long, env, default_value = "./manifests",
        help = "The directory containing package.json files that you want to mirror.")]
    manifests_path: String,

    #[arg(short, long, env, default_value = "./output",
        help = "The root directory where the mirror will be built")]
    output: String,

    #[arg(short, long, env, default_value_t = 8,
        help = "The number of concurrent downloads")]
    dl_threads: u8,

    #[arg(short, long, env, default_value = "https://registry.npmjs.org",
        help = "The NPM registry base url")]
    registry_url: Arc<String>,

    #[arg(short, long, env, default_value_t = false,
        help = "Verbose logging. Honestly still not very verbose, we don't want to be too spammy.")]
    verbose: bool,

    #[arg(short, long, env, default_value_t = false,
        help = "Changes the version matching from 'highest matching version' to 'any matching version'. This will pull down a LOT of packages for even the smallest manifest.")]
    greedy: bool,

    #[arg(long, env, default_value_t = false,
        help = "Don't download optional dependencies")]
    no_optional_deps: bool,

    #[arg(long, env, default_value_t = false,
        help = "Don't download dev-dependencies")]
    no_dev_deps: bool,

    #[arg(long, env, default_value_t = false,
        help = "Don't download peer-dependencies")]
    no_peer_deps: bool,

}

fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn log<M: Display>(msg: M) {
    println!("{} {msg}", now());
}