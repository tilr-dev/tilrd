#!/bin/bash
# Run manually as the logged-in user from the task folder, while the lag occurs.
# Collects evidence without restarting services or changing journal contents.
set -euo pipefail
[[ -f TASK.md && -f tilrd/diagnostic/fsevents-delay.c ]] || {
    echo 'Run from the task folder containing TASK.md and tilrd/.' >&2
    exit 2
}
sudo -v
stamp=$(date -u +%Y%m%dT%H%M%SZ)
prefix="$PWD/logs/privileged-$stamp"
probe_dir=$(mktemp -d /private/tmp/fsevents-delay-build.XXXXXX)
clang -Wall -Wextra -Werror -Wno-deprecated-declarations \
    tilrd/diagnostic/fsevents-delay.c -framework CoreServices -o "$probe_dir/native-delay"
(
    # Missing optional metadata must not prevent the primary acquisition.
    set +e
    date -u
    sw_vers
    ps -p "$(pgrep -x fseventsd)" -o pid,etime,time,pcpu,rss,state
    tmutil status
    mdutil -s /System/Volumes/Data
    df -h /private/tmp "$PWD"
    if [[ -d /Volumes/tilr-check ]]; then df -h /Volumes/tilr-check; fi
    sudo -n ls -ld /System/Volumes/Data/.fseventsd
    printf 'Journal directory entry count: '
    sudo -n ls -U /System/Volumes/Data/.fseventsd | wc -l
    exit 0
) > "$prefix-host.txt" 2>&1
sudo -n /usr/bin/sample fseventsd 10 10 -file "$prefix-sample.txt" > "$prefix-sample-command.txt" 2>&1 &
sample_pid=$!
sudo -n /usr/bin/fs_usage -w -f filesys -t 90 fseventsd native-delay > "$prefix-fs-usage.txt" 2>&1 &
trace_pid=$!
# Short all-process window identifies writers; may contain private file paths.
sudo -n /usr/bin/fs_usage -w -f filesys -t 15 > "$prefix-writers.txt" 2>&1 &
writers_pid=$!
"$probe_dir/native-delay" > "$prefix-native.txt" 2>&1 &
probe_pid=$!
status=0
for child in "$sample_pid" "$trace_pid" "$writers_pid" "$probe_pid"; do
    wait "$child" || status=1
done
{
    date -u
    ps -p "$(pgrep -x fseventsd)" -o pid,etime,time,pcpu,rss,state
} >> "$prefix-host.txt" 2>&1
printf 'Evidence: %s-* (status %s)\n' "$prefix" "$status"
exit "$status"
