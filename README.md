# CLIP OS updater

Client to update CLIP OS systems.

**WARNING: This is intended to be run only in a CLIP OS system! Running it as
root on a conventional system will result in data loss!**

## Threat model and general design

For a more detailed thread model, please see the
[security objectives](https://docs.clip-os.org/clipos/security.html) and the
[update model](https://docs.clip-os.org/clipos/updates.html).

## Environment setup

Common commands used to work on this project are available as recipes in
`justfile`. See [`just`](https://github.com/casey/just) for the full
documentation. You may install `just` with:

```
$ cargo install just
```

This project is currently being developed against the `1.34.2` version of Rust
(the version currently available in CLIP OS). This version will be
automatically installed by `cargo` using `rustup` but you may install it
manually using:

```
$ rustup install 1.34.2
```

The code is kept formatted using `cargo fmt`. To install the pre-commit hook:

```
$ ln -snf ../../.git_hooks/pre-commit .git/hooks/pre-commit
```

To avoid mistakes and potential data loss, setup a virtual machine for testing
using [Vagrant](https://www.vagrantup.com):

```
$ cd vagrant && vagrant up
```

Start a simple HTTP server to serve the static update content at webroot using
the serve recipe:

```
$ cargo install simple-http-server
$ just serve
```

If you are using `libvirt` and `firewalld`, you may have to open the 8000 port
using:

```
$ sudo firewall-cmd --zone=libvirt --add-port=8000/tcp
```

## Building and testing

Build the project and run the tests inside the virtual machine with:

```
$ just build
$ just test
```

## Update server webroot layout

Sample layout:

```
webroot
├── dist
│   └── 5.0.0-alpha.3
│       ├── clipos-core
│       ├── clipos-core.sig
│       ├── clipos-efiboot
│       └── clipos-efiboot.sig
└── update
    └── v1
        └── clipos
            └── version
```

Update payloads are stored in the `webroot/dist` directory. The naming scheme
is as follow: `<version>/<product>-<recipe>(.sig)`.

## Signing updates

Install [`minisign`](https://jedisct1.github.io/minisign/) or
[`rsign2`](https://github.com/jedisct1/rsign2) and use as example:

```
$ rsign sign \
    webroot/dist/5.0.0-alpha.3/clipos-core \
    -p test/keys/pub \
    -s test/keys/priv \
    -x webroot/dist/5.0.0-alpha.3/clipos-core.sig \
    -t "5.0.0-alpha.3"
```

* There is no password (empty password) for the test keys.
* The trusted comment must match the update version. This is verified by the
  client to prevent downgrade attacks.

## Update steps for the client

1. Retrieve the latest version available on the server:

   * GET https://update.clip-os.org/update/v1/clipos/version

   As the client sends its current version and its machine-id, the server
   determines the update channel associated with this machine-id and answers
   the latest version corresponding.

2. If the version is higher than the currently running version, the client
   retrieves update payloads from the server and verifies their authenticity:

   * Compare `version` with the current system version from `/etc/os-release`.
   * For core & efiboot packages:

     * GET https://update.clip-os.org/dist/<version>/<product>-<package>
     * GET https://update.clip-os.org/dist/<version>/<product>-<package>.sig

  * Validates the packages using the provided signature and the public key
    stored in the current system partition. Validate the packages versions
    using the signatures trusted comments.

3. Install update payloads:

   1. Validate system state and destinations for update payloads (empty space,
      available Logical Volumes, etc.)
   2. Remove the EFI binary installed in the EFI partition for the currently
      unused entry. This entry Core Logicial Volume will be overriden in the
      next step and must thus be made unbootable.
   3. Install (direct copy at block level) the new Core partition in the
      currently unused Logical Volume
   4. Install (file copy) the new EFI binary in the EFI partition.

## Planned improvements

See `TODO.md`.

## Gentoo ebuild & distfiles support

See the `package` recipe in `justfile`.
