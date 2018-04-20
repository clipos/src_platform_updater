// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright © 2019 ANSSI. All rights reserved.

use libmount::mountinfo::{MountPoint, Parser};
use minisign::PublicKey;
use minisign::SignatureBox;
use reqwest::Client;
use semver::Version;
use snafu::{ResultExt, Snafu};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str;

use crate::config::Remote;
use lvm;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Could not open or create file '{}': {}", filename.display(), source))]
    Io {
        filename: PathBuf,
        source: io::Error,
    },
    #[snafu(display("Could not read file '{}': {}", filename.display(), source))]
    Content {
        filename: PathBuf,
        source: io::Error,
    },
    #[snafu(display("Could not copy '{}' to '{}': {}", src.display(), dst.display(), source))]
    Copy {
        src: PathBuf,
        dst: PathBuf,
        source: std::io::Error,
    },
    #[snafu(display("Could not rename '{}' to '{}': {}", src.display(), dst.display(), source))]
    Rename {
        src: PathBuf,
        dst: PathBuf,
        source: std::io::Error,
    },
    #[snafu(display("Could not remove file '{}': {}", filename.display(), source))]
    Remove {
        filename: PathBuf,
        source: std::io::Error,
    },
    #[snafu(display("Unable to decode signature for file '{}': {}", filename.display(), source))]
    DecodeSignature {
        filename: PathBuf,
        source: minisign::PError,
    },
    #[snafu(display("Invalid signature for file '{}': {}", filename.display(), source))]
    InvalidSignature {
        filename: PathBuf,
        source: minisign::PError,
    },
    #[snafu(display("Invalid trusted comment for file '{}': {}", filename.display(), source))]
    InvalidTrustedComment {
        filename: PathBuf,
        source: minisign::PError,
    },
    #[snafu(display("Could not parse '{}' as valid version: {}", version, source))]
    InvalidVersion {
        version: String,
        source: semver::SemVerError,
    },
    #[snafu(display("Version from signature trusted comment does not match planned version for update: expecting '{}', got '{}'", expected, comment))]
    VersionMismatch {
        expected: semver::Version,
        comment: semver::Version,
    },
    #[snafu(display("HTTP request failed: {}", source))]
    HTTP { source: reqwest::Error },

    #[snafu(display("LVM command returned an error: {}", source))]
    Lvm { source: lvm::Error },

    #[snafu(display("Failed to call 'sync': {}", source))]
    Sync { source: io::Error },

    #[snafu(display("Failed to parse mountpoints from '/proc/self/mountinfo': {}", source))]
    Mountinfo {
        source: libmount::mountinfo::ParseError,
    },

    #[snafu(display("Could not list entry in directory '{}': {}", directory.display(), source))]
    ReadDir {
        directory: PathBuf,
        source: io::Error,
    },
    #[snafu(display("Invalid entry in directory '{}': {}", directory.display(), source))]
    DirEntry {
        directory: PathBuf,
        source: io::Error,
    },
    #[snafu(display("Could not found destination VG '{}'", vg))]
    VgNotFound { vg: String },
}

type Result<T> = std::result::Result<T, Error>;

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Error {
        Error::HTTP { source: err }
    }
}

impl From<lvm::Error> for Error {
    fn from(err: lvm::Error) -> Error {
        Error::Lvm { source: err }
    }
}

/// Meta structure to represent the current system state and ensure
/// that updates are installed in the correct order.
pub struct System {
    pub os_name: String,
    pub version: Version,

    core: Package,
    efiboot: Package,

    pubkey: PublicKey,

    download_cache: String,
}

/// The kind of package currently supported
#[derive(Debug, PartialEq)]
pub enum Kind {
    Core,
    Efiboot,
}

/// Represent a package (core, efiboot, etc.) to install on the system
#[derive(Debug)]
pub struct Package {
    kind: Kind,
    name: String,
    destination: String,
    size: Option<String>,
}

impl Package {
    pub fn new(kind: Kind, destination: &str, size: Option<String>) -> Package {
        let name = match kind {
            Kind::Core => String::from("core"),
            Kind::Efiboot => String::from("efiboot"),
        };
        Package {
            kind,
            name,
            destination: String::from(destination),
            size,
        }
    }
}

impl System {
    pub fn new(
        os_name: String,
        core: Package,
        efiboot: Package,
        version: Version,
        pubkey: PublicKey,
        download_cache: String,
    ) -> System {
        System {
            os_name,
            core,
            efiboot,
            version,
            pubkey,
            download_cache,
        }
    }

    /// Generate file name for package as stored in cache folder
    pub fn cache(&self, pkg: &Package) -> String {
        format!("{}/{}-{}", self.download_cache, &self.os_name, pkg.name)
    }
    /// Generate file name for package signature as stored in cache folder
    pub fn cache_sig(&self, pkg: &Package) -> String {
        format!("{}/{}-{}.sig", self.download_cache, &self.os_name, pkg.name)
    }

    /// Generate final installation destination file name for package
    pub fn dest(&self, pkg: &Package, v: &Version) -> String {
        match pkg.kind {
            Kind::Efiboot => format!("{}/{}-{}.efi", pkg.destination, &self.os_name, v),
            Kind::Core => format!("/dev/{}/{}_{}", pkg.destination, pkg.name, v),
        }
    }

    /// Generate URL to download package with given version
    pub fn url(&self, pkg: &Package, url: &str, v: &Version) -> String {
        format!("{}/{}/{}-{}", url, v, &self.os_name, pkg.name)
    }
    /// Generate URL to download package signature with given version
    pub fn url_sig(&self, pkg: &Package, url: &str, v: &Version) -> String {
        format!("{}/{}/{}-{}.sig", url, v, &self.os_name, pkg.name)
    }

    /// Update steps:
    /// 1. Download and validate efiboot
    /// 2. Download and validate core
    /// 3. Install core
    /// 4. Install efiboot
    pub fn update(&self, remote: Remote, version: Version) -> Result<()> {
        info!("Starting update to version '{}'", version);

        self.download(&self.efiboot, &remote, &version)?;
        self.download(&self.core, &remote, &version)?;

        self.install(&version)
    }

    /// Download given package with corresponding version from remote
    fn download(&self, pkg: &Package, r: &Remote, v: &Version) -> Result<()> {
        let file_url = &self.url(pkg, &r.dist_url, v);
        let file_dst = &self.cache(pkg);
        let sig_url = &self.url_sig(pkg, &r.dist_url, v);
        let sig_dst = &self.cache_sig(pkg);

        // Have we already downloaded a valid file?
        match self.validate(file_dst, sig_dst, v) {
            Err(_e) => debug!("invalid or incomplete precedent download"),
            Ok(()) => {
                info!("Reusing sucessfully downloaded and verified '{}'", file_dst);
                return Ok(());
            }
        }

        // Download requested file & its signature
        System::download_file(&file_url, &file_dst, r)?;
        System::download_file(&sig_url, &sig_dst, r)?;

        match self.validate(file_dst, sig_dst, v) {
            Err(e) => return Err(e),
            Ok(()) => info!("Sucessfully downloaded and verified '{}'", file_dst),
        }
        Ok(())
    }

    /// Download URL src to file dst using remote information
    fn download_file(src: &str, dst: &str, r: &Remote) -> Result<()> {
        debug!("Downloading '{}' to '{}'", src, dst);

        // Setup reqwest Client
        let client = Client::builder()
            .add_root_certificate(r.rootca.clone())
            .default_headers(r.headers.clone())
            .build()?;

        let mut res = client.get(src).send()?;
        let mut buf = File::create(dst).context(Io { filename: &dst })?;

        res.copy_to(&mut buf)?;
        Ok(())
    }

    /// Verify file using signature from sig, validating that the version match
    fn validate(&self, file: &str, sig: &str, v: &Version) -> Result<()> {
        let f = File::open(&file).context(Io { filename: file })?;
        let s = SignatureBox::from_file(sig).context(DecodeSignature { filename: sig })?;
        minisign::verify(&self.pubkey, &s, f, true, false)
            .context(InvalidSignature { filename: sig })?;

        let trusted_comment = s
            .trusted_comment()
            .context(InvalidTrustedComment { filename: sig })?;
        let version = Version::parse(&trusted_comment).context(InvalidVersion {
            version: trusted_comment,
        })?;

        if version != *v {
            return Err(Error::VersionMismatch {
                expected: (*v).clone(),
                comment: version,
            });
        }

        Ok(())
    }

    /// Install the system update
    fn install(&self, version: &Version) -> Result<()> {
        let core = &self.core;
        let efiboot = &self.efiboot;

        // Install LV image first as we do not want new boot entries to appear
        // until the core image is correctly installed
        info!(
            "Installing LV '{}' in VG '{}'",
            &core.name, &core.destination
        );

        let vg = match lvm::Vg::find_vg(&core.destination)? {
            Some(v) => v,
            None => {
                return Err(Error::VgNotFound {
                    vg: core.destination.clone(),
                })
            }
        };

        // Parse currently mounted devices
        let filename = "/proc/self/mountinfo";
        let mut f = File::open(filename).context(Io { filename })?;
        let mut content = String::new();
        f.read_to_string(&mut content)
            .context(Content { filename })?;
        let mut mountpoints: Vec<MountPoint> = Vec::new();
        for r in Parser::new(content.as_bytes()) {
            match r {
                Ok(m) => mountpoints.push(m),
                Err(e) => return Err(Error::Mountinfo { source: e }),
            }
        }

        // List all LV:
        // sudo lvs --noheadings main --reportformat json | jq '.report[].lv[].lv_name'
        // semver & find currently used lv and use the other
        // if only one LV, add a new one
        let lvs: Vec<lvm::Lv> = vg
            .list_lv()?
            .into_iter()
            .filter(|l| {
                let name = l.name();

                // Filter LVs starting with <pkg>_.*
                if !name.starts_with(format!("{}_", core.name).as_str()) {
                    return false;
                }

                // Filter LVs used for swap & state
                let mut s = name.split('_');
                let version = match s.nth(1) {
                    None => {
                        warn!("invalid LV name: nothing found after 'core_': '{}'", name);
                        return false;
                    }
                    Some(v) => v,
                };
                if version == "state" || version == "swap" {
                    debug!("ignoring LV: '{}'", name);
                    return false;
                }

                // Filter LVs with an incorrect version.
                // This should never happen but better be safe.
                let semver = match Version::parse(version) {
                    Err(_e) => {
                        warn!("could not parse '{}' as a version", version);
                        return false;
                    }
                    Ok(v) => v,
                };

                // Filter the currently in use version
                debug!("comparing: '{}' & '{}'", semver, self.version);
                if semver == self.version {
                    return false;
                }

                // Check that the LV is not in use before writing to it!
                // This should never happen, but we better be safe.
                let verity_name = format!("/dev/mapper/verity_{}_{}", name, semver);
                match mountpoints.iter().find(|m| {
                    if m.mount_point.to_os_string() != "/" {
                        debug!("Ignoring {:?} -> {:?}", &m.mount_source, &m.mount_point);
                        return false;
                    }
                    debug!("Looking at {:?} -> {:?}", &m.mount_source, &m.mount_point);
                    m.mount_source.to_os_string() == verity_name.as_str()
                }) {
                    Some(_mp) => {
                        warn!("ignoring: destination currently in use!");
                        return false;
                    }
                    None => debug!("proceeding: destination LV not in use"),
                };

                true
            })
            .collect();

        // TODO: Handle the case where we have more than 1 LV matching here
        if lvs.len() > 1 {
            warn!("More than one candidate LV found for {}", core.name);
        }
        // Pick an LV to install the image to
        let new_lv = format!("{}_{}", &core.name, &version);
        let mut lv = match lvs.first() {
            Some(l) => {
                info!("Installing over '{}'", l.name());
                l.clone()
            }
            None => {
                info!("Could not find a previous installation for '{}'", core.name);
                let size = match &core.size {
                    Some(s) => &s,
                    None => "500M",
                };
                vg.create_lv(&new_lv, size)?
            }
        };

        // To make sure that the system is in a consistent state, we must
        // remove boot entries before any destructive operation on the LVs.
        // Following steps:
        // * List all files in /mnt/efiboot/EFI/Linux
        // * Make sure to keep the currently booted version
        let current_efi = format!("{}-{}.efi", &self.os_name, &self.version);
        let mut files: Vec<PathBuf> = Vec::new();

        let dir = &efiboot.destination;
        for path in Path::new(dir)
            .read_dir()
            .context(ReadDir { directory: dir })?
        {
            let entry = match path {
                Err(e) => {
                    return Err(Error::DirEntry {
                        directory: PathBuf::from(dir),
                        source: e,
                    })
                }
                Ok(p) => p,
            };
            match entry.file_name().to_str() {
                None => warn!("Found invalid filename in efiboot"),
                Some(s) => {
                    if s != current_efi {
                        files.push(PathBuf::from(s));
                    }
                }
            };
        }

        // Warn if more than count files are remaining
        // TODO: Handle the case where we have more than 1 efi binary matching here
        if files.len() > 1 {
            warn!("More than one additionnal file found for {}", efiboot.name);
        }
        // Remove selected files
        for f in files {
            let filename = &Path::new(&efiboot.destination).join(f);
            debug!("Removing efiboot entry: {}", filename.display());
            fs::remove_file(filename).context(Remove { filename })?;
        }

        // We can now safely operate on unbootable LVs
        // First, rename the LV if necessary
        if lv.name() != new_lv {
            lv = lv.rename_to(new_lv)?;
        }

        // Copy the image content into the final LV
        // TODO: Check size before calling overwriting destination LV
        // TODO: Use casync with correct parameters
        let lv_path = &lv.path();
        let filename = &self.cache(core);
        let mut img = File::open(filename).context(Io { filename })?;
        let mut dev = OpenOptions::new()
            .read(true)
            .write(true)
            .open(lv_path)
            .context(Io { filename })?;
        io::copy(&mut img, &mut dev).context(Copy {
            src: filename,
            dst: lv_path,
        })?;

        // Install the EFI binary to create the boot entry
        info!(
            "Installing file '{}' to '{}'",
            efiboot.name, efiboot.destination
        );

        // First copy under a temporary name
        let filename = &self.cache(efiboot);
        let fullpath = &format!("{}.new", self.dest(efiboot, version));
        fs::copy(filename, fullpath).context(Copy {
            src: filename,
            dst: fullpath,
        })?;

        // Call sync to avoid partially written files
        Command::new("sync")
            .spawn()
            .context(Sync {})?
            .wait()
            .context(Sync {})?;

        // Rename to the final name
        let final_path = &self.dest(efiboot, version);
        fs::rename(fullpath, final_path).context(Rename {
            src: fullpath,
            dst: final_path,
        })?;

        // As the update completed successfully, we can now remove temporary files.
        // Errors are ignored here as they are not fatal and should never happen.
        fs::remove_file(self.cache(core))
            .unwrap_or_else(|e| warn!("Could not remove temporary file: {}", e));
        fs::remove_file(self.cache_sig(core))
            .unwrap_or_else(|e| warn!("Could not remove temporary file: {}", e));
        fs::remove_file(self.cache(efiboot))
            .unwrap_or_else(|e| warn!("Could not remove temporary file: {}", e));
        fs::remove_file(self.cache_sig(efiboot))
            .unwrap_or_else(|e| warn!("Could not remove temporary file: {}", e));

        Ok(())
    }
}
