//! `lull` binary. One executable dispatched by `argv[0]` / subcommand: `lull daemon`,
//! `lull <cli-cmd>` and the `lull-nm-host` symlink (contract §3). Implemented in WP1.5
//! and WP2.1; M0 only reports the protocol version.

fn main() {
    let v = lull_protocol::PROTOCOL_VERSION;
    println!(
        "lull {} (protocol {}.{})",
        env!("CARGO_PKG_VERSION"),
        v.major,
        v.minor
    );
}
