use clap::Parser as _;

mod api;
mod cache;
mod cli;
mod config;
mod error;

use api::Client;
use cli::{Cli, Command};
use config::Config;

macro_rules! crash_if_err {
    ($exp:expr) => {
        match $exp {
            Ok(result) => result,
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
    };
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let cli = Cli::parse();

    let mut cfg = crash_if_err! { cli.config_file().and_then(Config::load) };
    cfg.override_with(&cli);

    let cmd = cli.into();
    cfg.override_with(&cmd);

    let client = Client::new(cfg).await;

    match cmd {
        Command::Shorten(args) => {
            let bitlink = crash_if_err! { client.shorten(args.url).await };
            println!("{}", bitlink.link);
        }
    }
}
