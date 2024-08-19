use std::pin::pin;

use clap::Parser as _;
use futures_util::stream::{self, StreamExt as _};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

mod api;
mod cache;
mod cli;
mod config;
mod error;
mod io;

use api::Client;
use cli::{Cli, Command, Ordering};
use config::{Config, APP};

macro_rules! crash_if_err {
    ($exp:expr) => {
        match $exp {
            Ok(result) => result,
            Err(error) => {
                eprintln!("{APP}: {error}");
                std::process::exit(1);
            }
        }
    };
}

fn setup_tracing() {
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_file(true)
        .with_line_number(true)
        .compact();

    let env_filter = tracing_subscriber::EnvFilter::from_default_env();

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(env_filter)
        .init();
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    setup_tracing();

    let cli = Cli::parse();

    let mut cfg = crash_if_err! { cli.config_file().and_then(Config::load) };
    cfg.override_with(&cli);

    let cmd = cli.into();
    cfg.override_with(&cmd);

    let client = Client::new(cfg).await;

    match cmd {
        Command::Shorten(args) => {
            let urls = if args.urls.is_empty() {
                let Some(urls) = io::read_input::<url::Url>() else {
                    return;
                };
                urls.map(|url| crash_if_err!(url)).left_stream()
            } else {
                stream::iter(args.urls).right_stream()
            };

            let mut results = pin!(client.shorten(urls, args.ordering));

            match args.ordering {
                Ordering::Ordered => {
                    while let Some(result) = results.next().await {
                        let bitlink = crash_if_err! { result };
                        println!("{}", bitlink.link);
                    }
                }

                Ordering::Unordered => {
                    while let Some(result) = results.next().await {
                        let bitlink = crash_if_err! { result };
                        println!("{}\t{}", bitlink.link, bitlink.long_url);
                    }
                }
            }
        }
    }
}
