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

# Debug
# set -x

main() {
    readonly disk_image="/home/vagrant/disk.img"

    # Create a fake CLIP OS installation
    fallocate -l 4G "${disk_image}"

    sgdisk --zap-all "${disk_image}"

    sgdisk --new=partnum:1M:513M "${disk_image}"
    sgdisk --typecode=1:C12A7328-F81F-11D2-BA4B-00A0C93EC93B "${disk_image}"
    sgdisk --change-name=1:EFI "${disk_image}"

    sgdisk --largest-new=2 "${disk_image}"
    sgdisk --typecode=2:E6D6D379-F507-44C2-A23C-238F2A3DF928 "${disk_image}"
    sgdisk --change-name=2:LVM "${disk_image}"

    sudo losetup -f "${disk_image}"

    readonly device="/dev/loop0"

    sudo partprobe "${device}"

    readonly product_version="5.0.0-alpha.1"
    readonly vg_name="mainvg"
    readonly core_lv_name="core_${product_version}"

    sudo mkfs.vfat "${device}p1" -n EFI
    sudo pvcreate "${device}p2"
    sudo vgcreate "${vg_name}" "${device}p2"

    sudo vgchange -ay
    sudo vgscan

    sudo lvcreate -L 500M -n "${core_lv_name}" "${vg_name}"
    sudo lvcreate -L 512M -n core_state        "${vg_name}"
    sudo lvcreate -L 512M -n core_swap         "${vg_name}"

    sudo mkdir -p /mnt/efiboot
    sudo mount "${device}p1" /mnt/efiboot
    sudo mkdir -p /mnt/efiboot/EFI/Linux
}

main
