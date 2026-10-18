#!/bin/sh
# Entrypoint of the Linux container (scripts/linux.Dockerfile): runs the command as `runner`,
# a non-root user with a passwd entry, a home directory and a login shell, as on the GitHub
# runner. Root differs from such a user in ways the tests notice (file permissions are not
# enforced for root; the login shell and HOME come from passwd), so the suite runs as one.
# The user takes the uid/gid that owns /src (the host user on a Linux host, so files written
# to /src, e.g. dist/, belong to them), or 1000 when /src shows as root (Docker Desktop).
# TILR_LINUX_USER=root runs the command as root instead.
set -eu

if [ "$(id -u)" != 0 ] || [ "${TILR_LINUX_USER:-runner}" = root ]; then
    exec "$@"
fi

uid=$(stat -c %u /src)
gid=$(stat -c %g /src)
if [ "$uid" = 0 ]; then
    uid=1000
    gid=1000
fi
groupadd -o -g "$gid" runner
useradd -o -m -u "$uid" -g "$gid" -s /bin/bash runner
# The cache volumes may have been filled by root (earlier runs); hand them over once.
for dir in "$CARGO_TARGET_DIR" "$CARGO_HOME/registry"; do
    mkdir -p "$dir"
    if [ "$(stat -c %u "$dir")" != "$uid" ]; then
        chown -R "$uid:$gid" "$dir"
    fi
done
exec setpriv --reuid="$uid" --regid="$gid" --init-groups \
    env HOME=/home/runner USER=runner LOGNAME=runner SHELL=/bin/bash "$@"
