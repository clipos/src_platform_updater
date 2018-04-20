// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright © 2019 ANSSI. All rights reserved.

//! This is an in-progress and incomplete wrapper around the LVM2 command line
//! tools for usage in Rust.
//!
//! We do not use the
//! [liblvm2app API](https://github.com/lvmteam/lvm2/blob/stable-2.02/liblvm/lvm2app.h)
//! as it has been deprecated since Sep 19, 2017 (see upstream commit
//! [4cbacf6](https://github.com/lvmteam/lvm2/commit/4cbacf6bac47227ec3460da1619ab29557f2c89b)).
//!
//! In the future, this will be rewritten to use the official D-Bus API with
//! [lvmdbusd](http://man7.org/linux/man-pages/man8/lvmdbusd.8.html) or the
//! [libblockdev](https://github.com/storaged-project/libblockdev) API.

#![forbid(unsafe_code)]

#[macro_use]
extern crate log;
#[macro_use]
extern crate serde_derive;
extern crate serde;
extern crate serde_json;
extern crate snafu;

use serde::de::DeserializeOwned;
use snafu::{OptionExt, ResultExt, Snafu};
use std::ffi::OsStr;
use std::process::Command;
use std::str;

/// Represent a Volume Group
#[derive(Clone)]
pub struct Vg {
    pub name: String,
}

/// Used to automatically parse LVM JSON output
#[derive(Deserialize, Debug)]
struct JsonReportVgs {
    report: Vec<JsonReportVgsList>,
}

/// Used to automatically parse LVM JSON output
#[derive(Deserialize, Debug)]
struct JsonReportVgsList {
    vg: Vec<JsonReportVgsVg>,
}

/// Used to automatically parse LVM JSON output
#[derive(Deserialize, Debug)]
struct JsonReportVgsVg {
    vg_name: String,
    pv_count: String,
    lv_count: String,
    snap_count: String,
    vg_attr: String,
    vg_size: String,
    vg_free: String,
}

/// Represent a Logical Volume
#[derive(Clone)]
pub struct Lv {
    name: String,
    vg: Vg,
}

/// Used to automatically parse LVM JSON output
#[derive(Deserialize, Debug)]
struct JsonReportLvs {
    report: Vec<JsonReportLvsList>,
}

/// Used to automatically parse LVM JSON output
#[derive(Deserialize, Debug)]
struct JsonReportLvsList {
    lv: Vec<JsonReportLvsLv>,
}

/// Used to automatically parse LVM JSON output
#[derive(Deserialize, Debug)]
struct JsonReportLvsLv {
    lv_name: String,
    vg_name: String,
    lv_attr: String,
    lv_size: String,
    pool_lv: String,
    origin: String,
    data_percent: String,
    metadata_percent: String,
    move_pv: String,
    mirror_log: String,
    copy_percent: String,
    convert_lv: String,
}

/// Library specific Error type
#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Could not execute LVM command '{:?}': {}", command, source))]
    ProcessExec {
        command: Command,
        source: std::io::Error,
    },
    #[snafu(display("LVM command '{}' returned an error: {}", command, message))]
    LvmCommand { command: String, message: String },
    #[snafu(display(
        "Could not parse LVM command '{}' stderr output as UTF-8: {}, output: {:?}",
        command,
        source,
        output
    ))]
    StdErrUTF8 {
        command: String,
        output: Vec<u8>,
        source: std::string::FromUtf8Error,
    },
    #[snafu(display(
        "Could not parse LVM command '{}' stdout output as UTF-8: {}, output: {:?}",
        command,
        source,
        output
    ))]
    StdOutUTF8 {
        command: String,
        output: Vec<u8>,
        source: std::string::FromUtf8Error,
    },
    #[snafu(display(
        "Could not parse LVM command '{}' JSON report: {}, output: {:?}",
        command,
        source,
        output
    ))]
    ReportParsing {
        command: String,
        output: Vec<u8>,
        source: serde_json::error::Error,
    },
    #[snafu(display("Unexpected format for the JSON report for LVM command '{}'", command))]
    UnexpectedReportFormat { command: String },
}

/// Library specific Result type
pub type Result<T> = std::result::Result<T, Error>;

/// Wrapper to run LVM commands that do not return meaningful JSON output
fn command<I, S>(cmd: &str, args: Option<I>) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(cmd);

    if let Some(a) = args {
        command.args(a);
    }

    let cmd_fmt = format!("{:?}", &command);
    debug!("will run: {}", cmd_fmt);

    let output = command.output().context(ProcessExec { command })?;

    if !output.status.success() {
        let message = String::from_utf8(output.stderr.clone()).context(StdErrUTF8 {
            command: cmd_fmt.clone(),
            output: output.stderr,
        })?;
        return Err(Error::LvmCommand {
            command: cmd_fmt,
            message,
        });
    }

    let message = String::from_utf8(output.stdout.clone()).context(StdOutUTF8 {
        command: cmd_fmt,
        output: output.stdout,
    })?;
    debug!("output: {}", message);
    Ok(message)
}

/// Wrapper to run LVM commands with the JSON reporting format and parse its output
fn command_json<T, I, S>(cmd: &str, args: Option<I>) -> Result<T>
where
    T: std::fmt::Debug + DeserializeOwned,
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(cmd);
    command.args(&["--reportformat", "json"]);

    if let Some(a) = args {
        command.args(a);
    }

    let cmd_fmt = format!("{:?}", &command);
    debug!("will run: {}", cmd_fmt);

    let output = command.output().context(ProcessExec { command })?;

    if !output.status.success() {
        let message = String::from_utf8(output.stderr.clone()).context(StdErrUTF8 {
            command: cmd_fmt.clone(),
            output: output.stderr,
        })?;
        return Err(Error::LvmCommand {
            command: cmd_fmt,
            message,
        });
    }

    let report: T = serde_json::from_slice(&output.stdout).context(ReportParsing {
        command: cmd_fmt,
        output: output.stdout,
    })?;
    debug!("json output: {:?}", report);
    Ok(report)
}

impl Lv {
    /// Get the Logical Volume name
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// Get the full path to a Logical Volume in /dev
    pub fn path(&self) -> String {
        format!("/dev/{}/{}", &self.vg.name, &self.name)
    }

    /// Rename a Logical Volume
    pub fn rename_to(&self, dest: String) -> Result<Lv> {
        debug!("Renaming LV '{}' to '{}'", &self.name, dest);

        command::<&[&str], _>("lvrename", Some(&[&self.vg.name, &self.name, &dest]))?;

        debug!("Renamed LV '{}' to '{}'", &self.name, dest);

        Ok(Lv {
            name: dest,
            vg: Vg {
                name: self.vg.name.clone(),
            },
        })
    }
}

impl Vg {
    /// List all currently available Volume Group
    pub fn vgs() -> Result<Vec<Vg>> {
        debug!("Listing all available VG");

        let command = "vgs";
        let vg_list = match command_json::<JsonReportVgs, &[&str], _>(command, None) {
            Err(e) => return Err(e),
            Ok(l) => l,
        };

        Ok(vg_list
            .report
            .first()
            .context(UnexpectedReportFormat { command })?
            .vg
            .iter()
            .map(|vg| Vg {
                name: vg.vg_name.clone(),
            })
            .collect::<Vec<Vg>>())
    }

    /// Find a Volume Group by its name
    pub fn find_vg(name: &str) -> Result<Option<Vg>> {
        debug!("Looking for VG '{}'", &name);

        let command = "vgs";
        let vg_list = match command_json::<JsonReportVgs, &[&str], _>(command, None) {
            Err(e) => return Err(e),
            Ok(l) => l,
        };

        for vg in &vg_list
            .report
            .first()
            .context(UnexpectedReportFormat { command })?
            .vg
        {
            if vg.vg_name == name {
                debug!("Found VG '{}'", name);
                return Ok(Some(Vg {
                    name: vg.vg_name.clone(),
                }));
            }
        }

        Ok(None)
    }

    /// List all Logical Volume in a Volume Group
    pub fn list_lv(&self) -> Result<Vec<Lv>> {
        debug!("Listing all available LV for VG '{}'", &self.name);

        let command = "lvs";
        let lv_list = match command_json::<JsonReportLvs, &[&str], _>(command, Some(&[&self.name]))
        {
            Err(e) => return Err(e),
            Ok(l) => l,
        };

        Ok(lv_list
            .report
            .first()
            .context(UnexpectedReportFormat { command })?
            .lv
            .iter()
            .map(|lv| {
                debug!("Found LV: {}", lv.lv_name.clone());
                Lv {
                    name: lv.lv_name.clone(),
                    vg: Vg {
                        name: self.name.clone(),
                    },
                }
            })
            .collect::<Vec<Lv>>())
    }

    /// Find a Logical Volume by its name
    pub fn find_lv(&self, name: &str) -> Result<Option<Lv>> {
        debug!("Looking for LV '{}' in VG '{}'", &name, &self.name);

        let command = "lvs";
        let lv_list = match command_json::<JsonReportLvs, &[&str], _>(command, Some(&[&self.name]))
        {
            Err(e) => return Err(e),
            Ok(l) => l,
        };

        for lv in &lv_list
            .report
            .first()
            .context(UnexpectedReportFormat { command })?
            .lv
        {
            debug!("Looking at '{}'/'{}'", lv.vg_name, lv.lv_name);
            if lv.lv_name == name && lv.vg_name == self.name {
                debug!("Found LV '{}'", name);
                return Ok(Some(Lv {
                    name: lv.lv_name.clone(),
                    vg: Vg {
                        name: self.name.clone(),
                    },
                }));
            }
        }

        debug!("Could not find LV '{}' in VG '{}'", name, &self.name);
        Ok(None)
    }

    /// Create a new Logical Volume in the given Volume Group
    pub fn create_lv(&self, name: &str, size: &str) -> Result<Lv> {
        debug!(
            "Creating LV '{}' with size '{}' in VG '{}'",
            name, size, &self.name
        );

        command::<&[&str], _>("lvcreate", Some(&["-L", size, "-n", name, &self.name]))?;

        debug!(
            "Created LV '{}' with size '{}' in VG '{}'",
            name, size, &self.name
        );

        Ok(Lv {
            name: String::from(name),
            vg: Vg {
                name: self.name.clone(),
            },
        })
    }
}
