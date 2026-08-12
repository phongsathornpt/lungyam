use std::env;

use lungyam_proxy::{run, runtime_banner};

const DEFAULT_LISTEN: &str = "0.0.0.0:8080";
const DEFAULT_UPSTREAM: &str = "127.0.0.1:3000";

fn main() {
    let listen = env::var("LUNGYAM_LISTEN").unwrap_or_else(|_| DEFAULT_LISTEN.to_owned());
    let upstream = env::var("LUNGYAM_UPSTREAM").unwrap_or_else(|_| DEFAULT_UPSTREAM.to_owned());

    println!("{} listening on {listen} -> {upstream}", runtime_banner());
    run(&listen, &upstream);
}
