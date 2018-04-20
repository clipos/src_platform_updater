// SPDX-License-Identifier: LGPL-2.1-or-later
// Copyright © 2019 ANSSI. All rights reserved.

use minisign::PublicKey;
use os_release::OsRelease;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::{Certificate, Client};
use semver::Version;
use snafu::{OptionExt, ResultExt, Snafu};
use std::fs;
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use crate::system::{Kind, Package, System};

/// Required information to get update from a remote source
pub struct Remote {
    pub update_url: String,
    pub dist_url: String,
    pub rootca: Certificate,
    pub headers: HeaderMap,
}

/// Used to parse `config.toml` configuration files
#[derive(Deserialize, Debug)]
pub struct TomlConfig {
    os_name: String,
    core: TomlCore,
    efiboot: TomlEfiboot,
}

/// Used to parse `config.toml` configuration files
#[derive(Deserialize, Debug)]
pub struct TomlCore {
    destination: String,
    size: String,
}

/// Used to parse `config.toml` configuration files
#[derive(Deserialize, Debug)]
pub struct TomlEfiboot {
    destination: String,
}

/// Used to parse `remote.toml` configuration files
#[derive(Deserialize, Debug)]
pub struct TomlRemote {
    update_url: String,
    dist_url: String,
}

/// Used to parse `version.toml` remote configuration files
#[derive(Deserialize, Debug)]
pub struct TomlVersion {
    version: String,
}

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Could not open file '{}': {}", filename.display(), source))]
    MissingFile {
        filename: PathBuf,
        source: std::io::Error,
    },
    #[snafu(display("Could not read file '{}': {}", filename.display(), source))]
    InvalidFile {
        filename: PathBuf,
        source: std::io::Error,
    },
    #[snafu(display("Could not parse TOML '{}': {}", filename.display(), source))]
    InvalidToml {
        filename: PathBuf,
        source: toml::de::Error,
    },
    #[snafu(display("Could not parse received TOML: {}", source))]
    InvalidRemoteToml { source: toml::de::Error },
    #[snafu(display("Could not parse '{}' as valid version: {}", version, source))]
    InvalidVersion {
        version: String,
        source: semver::SemVerError,
    },
    #[snafu(display("Could not read a valid machine-id from '{}'", filename))]
    InvalidMachineId { filename: String },
    #[snafu(display("Invalid value for HTTP header '{}': {}", value, source))]
    InvalidHeader {
        value: String,
        source: reqwest::header::InvalidHeaderValue,
    },
    #[snafu(display("Could not parse public key from '{}': {}", filename.display(), source))]
    InvalidPublicKey {
        filename: PathBuf,
        source: minisign::PError,
    },
    #[snafu(display("Could not parse root certificate from '{}': {}", filename.display(), source))]
    InvalidCertificate {
        filename: PathBuf,
        source: reqwest::Error,
    },
    #[snafu(display("HTTP request failed: {}", source))]
    HTTP { source: reqwest::Error },
}

impl From<reqwest::Error> for Error {
    fn from(err: reqwest::Error) -> Error {
        Error::HTTP { source: err }
    }
}

impl From<toml::de::Error> for Error {
    fn from(err: toml::de::Error) -> Error {
        Error::InvalidRemoteToml { source: err }
    }
}

type Result<T> = std::result::Result<T, Error>;

/// Parse the following configuration files from the configuration folder:
///   * config.toml
///   * pubkey
///   * remote.toml
///   * rootca
///
/// Retrieve information from the following configuration files:
///   * /etc/os-release
///   * /etc/machine-id
pub fn parse(config: PathBuf, remote: PathBuf, tmp: String) -> Result<(System, Remote)> {
    let filename = &config.join("config.toml");
    let mut content = String::new();
    File::open(&filename)
        .context(MissingFile { filename })?
        .read_to_string(&mut content)
        .context(InvalidFile { filename })?;
    let c: TomlConfig = toml::from_str(&content).context(InvalidToml { filename })?;
    debug!("Read {}:\n{:#?}", filename.display(), &c);

    // Get current version from /etc/os-release
    let version_id = OsRelease::new()
        .context(InvalidFile {
            filename: "/etc/os-release",
        })?
        .version_id;
    let mut version = Version::parse(&version_id).context(InvalidVersion {
        version: version_id,
    })?;
    // Remove any build information if any to ignore '+instrumented' build markers
    version.build.clear();
    version.build.shrink_to_fit();

    info!("Currently on '{}', version '{}'", c.os_name, version);

    let filename = &config.join("pubkey");
    let pubkey = PublicKey::from_file(filename).context(InvalidPublicKey { filename })?;
    debug!("Read public key from {}", filename.display());

    let filename = &remote.join("remote.toml");
    let mut content = String::new();
    File::open(&filename)
        .context(MissingFile { filename })?
        .read_to_string(&mut content)
        .context(InvalidFile { filename })?;
    let r: TomlRemote = toml::from_str(&content).context(InvalidToml { filename })?;
    debug!("Read {}:\n{:#?}", filename.display(), &r);
    info!("Looking for updates at '{}'", r.update_url);

    let filename = &remote.join("rootca.pem");
    let rootca = Certificate::from_pem(&fs::read(&filename).context(InvalidFile { filename })?)
        .context(InvalidCertificate { filename })?;
    debug!("Read {}", filename.display());

    let core = Package::new(Kind::Core, &c.core.destination, Some(c.core.size));
    let efiboot = Package::new(Kind::Efiboot, &c.efiboot.destination, None);

    // Get machine-id from /etc/machine-id
    let filename = "/etc/machine-id";
    let mut machine_id = String::new();
    File::open(&filename)
        .context(MissingFile { filename })?
        .read_to_string(&mut machine_id)
        .context(InvalidFile { filename })?;
    let machine_id = machine_id
        .lines()
        .next()
        .context(InvalidMachineId { filename })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        "clipos-machineid",
        HeaderValue::from_str(machine_id).context(InvalidHeader { value: machine_id })?,
    );
    headers.insert(
        "clipos-version",
        HeaderValue::from_str(&format!("{}", version))
            .context(InvalidHeader { value: machine_id })?,
    );

    Ok((
        System::new(c.os_name, core, efiboot, version, pubkey, tmp),
        Remote {
            update_url: r.update_url,
            dist_url: r.dist_url,
            rootca,
            headers,
        },
    ))
}

impl Remote {
    pub fn check_update(&self, system: &System) -> Result<Option<Version>> {
        // Setup reqwest Client
        let client = Client::builder()
            .add_root_certificate(self.rootca.clone())
            .default_headers(self.headers.clone())
            .build()?;

        // Get {update_url}/{os_name}/version
        let url = format!("{}/{}/version", self.update_url, system.os_name);
        debug!("GET {}", &url);
        let body = client.get(&url).send()?.text()?;
        debug!("body = {:?}", body);

        // Parse response
        let v: TomlVersion = toml::from_str(&body)?;
        let version = v.version;
        debug!("Remote version: {}", version);

        // Compare versions
        let remote_version = Version::parse(&version).context(InvalidVersion { version })?;
        debug!(
            "local version: '{}' | remote version: '{}'",
            system.version, remote_version
        );
        if system.version >= remote_version {
            return Ok(None);
        }

        Ok(Some(remote_version))
    }
}
