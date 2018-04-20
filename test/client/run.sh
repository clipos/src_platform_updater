#!/usr/bin/env bash
# SPDX-License-Identifier: LGPL-2.1-or-later
# Copyright © 2019 ANSSI. All rights reserved.

# Safety settings: do not remove!
# Do not use 'set -o errexit' as some setup commands may fail
set -o nounset -o pipefail
# Debug
# set -x

if [[ ! -d "/vagrant" ]]; then
    echo "We are not running inside a test virtual machine!"
    echo "This is dangerous. Exiting!"
    exit 1
fi

cleanup() {
    # Cleanup LVs created during previous runs
    sudo lvremove mainvg/core_5.0.0-alpha.0 -y &> /dev/null
    sudo lvremove mainvg/core_5.0.0-alpha.2 -y &> /dev/null
    sudo lvremove mainvg/core_5.0.0-alpha.3 -y &> /dev/null
    sudo lvremove mainvg/core_5.0.0-alpha.4 -y &> /dev/null
    sudo rm -f /mnt/efiboot/EFI/Linux/clipos-*.efi &> /dev/null
    sudo touch /mnt/efiboot/EFI/Linux/clipos-5.0.0-alpha.1.efi
}

test_header() {
    echo ""
    echo "###############################################"
    echo "# ${1}"
    echo "###############################################"
}

test_check() {
    rm -f /tmp/lvs /tmp/tree

    echo "###############################################"

    sudo lvs > /tmp/lvs
    ls /mnt/efiboot/EFI/Linux > /tmp/efiboot

    # Checks may fail
    set +e
    diff -u ${HOME}/output/lvs     /tmp/lvs
    diff -u ${HOME}/output/efiboot /tmp/efiboot
    # TODO: fail if non empty diff
    set -e

    echo "# OK"

    echo "###############################################"
}

main() {
    readonly disk_image="/home/vagrant/disk.img"

    if [[ ! -f "${disk_image}" ]]; then
        ./setup_fake_clipos_disk_image.sh
    fi

    readonly device="/dev/loop0"

    if [[ ! -e ${device} ]]; then
        sudo losetup -f "${disk_image}"
        sudo partprobe "${device}"
        sudo systemctl restart lvm2-lvmetad.service
        sudo vgscan > /dev/null
        sudo vgchange -ay > /dev/null
        sudo vgscan > /dev/null
        sudo mkdir -p /mnt/efiboot
        sudo mount "${device}p1" /mnt/efiboot
    fi

    # Default command
    CMD="sudo ./updater -c config -r remote -t /tmp"
    # Full debug command
    # CMD="sudo RUST_BACKTRACE=1 ./updater -v -c config -r remote -t /tmp"

    cleanup

    # No need to keep going if a test fails now
    set -e

    # First run to test LV creation (first update case)
    test_header "LV creation"
    ${CMD}
    test_check

    # Second run to test LV renaming (second normal update case)
    test_header "LV renaming (normal update)"
    sudo lvrename mainvg core_5.0.0-alpha.3 core_5.0.0-alpha.0 &> /dev/null
    sudo mv /mnt/efiboot/EFI/Linux/clipos-5.0.0-alpha.{3,0}.efi
    ${CMD}
    test_check

    # Third run to test LV renaming with a higher version available (user rollback)
    test_header "LV renaming (user rollback)"
    sudo lvrename mainvg core_5.0.0-alpha.3 core_5.0.0-alpha.2 &> /dev/null
    sudo mv /mnt/efiboot/EFI/Linux/clipos-5.0.0-alpha.{3,2}.efi
    ${CMD}
    test_check

    # Fourth run to test LV renaming with a version rollback (edge case)
    test_header "LV renaming (edge case)"
    sudo lvrename mainvg core_5.0.0-alpha.3 core_5.0.0-alpha.4 &> /dev/null
    sudo mv /mnt/efiboot/EFI/Linux/clipos-5.0.0-alpha.{3,4}.efi
    ${CMD}
    test_check
}

main
