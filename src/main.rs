// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright © 2019 ANSSI. All rights reserved.

//! Command line client to update CLIP OS systems.
//!
//! **WARNING: This is intended to be run only in a CLIP OS system! Running it
//! as root on a conventional system will result in data loss!**
//!
//! The threat model and design for this updater is described in the [security
//! objectives](https://docs.clip-os.org/clipos/security.html) and [update
//! model](https://docs.clip-os.org/clipos/updates.html) documentation of the
//! CLIP OS project.
//!
//! See also the [README](https://github.com/clipos/src_platform_updater) for
//! more information.

#![forbid(unsafe_code)]

extern crate env_logger;
#[macro_use]
extern crate log;
extern crate reqwest;
#[macro_use]
extern crate serde_derive;
extern crate libmount;
extern crate lvm;
extern crate minisign;
extern crate os_release;
extern crate semver;
extern crate serde;
extern crate snafu;
extern crate structopt;
extern crate toml;

mod config;
mod system;

use log::LevelFilter;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::process::exit;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
#[structopt(name = "updater", about = "CLIP OS updater")]
struct Opt {
    /// Verbose mode (-v, -vv, -vvv, etc.)
    #[structopt(short = "v", long = "verbose", parse(from_occurrences))]
    verbose: u8,

    /// Path to system configuration files (config.toml & pubkey)
    #[structopt(
        short = "c",
        long = "config",
        parse(from_os_str),
        default_value = "/usr/lib/updater"
    )]
    config: PathBuf,

    /// Path to remote configuration files (remote.toml & rootca.pem)
    #[structopt(
        short = "r",
        long = "remote",
        parse(from_os_str),
        default_value = "/etc/updater"
    )]
    remote: PathBuf,

    /// Path to temporary folder used to download update payloads
    #[structopt(
        short = "t",
        long = "tmp",
        parse(from_str),
        default_value = "/var/lib/updater"
    )]
    tmp: String,
}

fn main() {
    // Parse command line arguments and set log level.
    // We do not filter higher than Info by default.
    let opt = Opt::from_args();

    let level = match opt.verbose {
        0 => LevelFilter::Info,
        1 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    env_logger::Builder::new()
        .filter(None, level)
        .default_format_timestamp(false)
        .init();

    info!("Starting updater");

    let (system, remote) = match config::parse(opt.config, opt.remote, opt.tmp) {
        Err(e) => {
            error!("{}", e);
            info!("Exiting");
            exit(1);
        }
        Ok(c) => c,
    };

    let version = match remote.check_update(&system) {
        Err(e) => {
            error!("{}", e);
            info!("Exiting");
            exit(1);
        }
        Ok(r) => match r {
            None => {
                info!("No update found");
                info!("Exiting");
                exit(0);
            }
            Some(v) => v,
        },
    };

    // Apply update payloads and install the new EFI boot entries
    match system.update(remote, version) {
        Err(e) => {
            error!("{}", e);
            info!("Exiting");
            exit(1);
        }
        Ok(()) => info!("Successfully updated!"),
    }

    // TODO: Inform the user that an update is ready and a reboot is required
    // For now we drop an empty file in a specific path in /run
    // The systemd unit will not trigger if this file exists, thus avoiding repeated
    // updates in a loop.
    let marker = "/run/update_ready";
    match OpenOptions::new().create(true).write(true).open(&marker) {
        Ok(_f) => debug!("Touched '{}'", &marker),
        Err(e) => warn!("Could not touch '{}': {}", &marker, e),
    };

    info!("Exiting");
    exit(0);
}
