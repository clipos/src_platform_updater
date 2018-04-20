#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# Copyright © 2019 ANSSI. All rights reserved.

# Safety settings: do not remove!
set -o errexit -o nounset -o pipefail

if [[ ! -d "/vagrant" ]]; then
    echo "We are not running inside a test virtual machine!"
    echo "This is dangerous. Exiting!"
    exit 1
fi

# Install required dependencies & development tools
deps=(
    "bash-completion"
    "dosfstools"
    "gptfdisk"
    "lvm2"
    "parted"
    "squashfs-tools"
    "tree"
)
sudo pacman -Syu --noconfirm ${deps[@]}

# Setup static IP for update.clip-os.org test domain
echo "$(hostname -i  | cut -d'.' -f '1-3').1 update.clip-os.org" | sudo tee -a /etc/hosts

# Setup fake version in os-release
echo "VERSION_ID=\"5.0.0-alpha.1\"" | sudo tee -a /etc/os-release

sudo reboot
