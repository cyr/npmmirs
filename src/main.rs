
use std::{fmt::Display, process::exit};

use clap::{command, arg, Parser};
use downloader::Downloader;
use mirror::mirror;

mod downloader;
mod error;
mod progress;
mod checksum;
mod mirror;
mod metadata;

#[tokio::main]
async fn main() {
    dotenv::dotenv().ok();

    let opts = CliOpts::parse();

    let downloader = Downloader::build(opts.dl_threads);

    log("Mirroring started");
    match mirror(&opts, downloader).await {
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


#[derive(Parser)]
#[command(author, version, about)]
struct CliOpts {
    #[arg(short, long, env, default_value = "./manifests")]
    manifests_path: String,

    #[arg(short, long, env, default_value = "./output")]
    output: String,

    #[arg(short, long, env, default_value_t = 8)]
    dl_threads: u8,

    #[arg(short, long, env, default_value = "https://registry.npmjs.org")]
    registry_url: String,
}

fn now() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn log<M: Display>(msg: M) {
    println!("{} {msg}", now());
}