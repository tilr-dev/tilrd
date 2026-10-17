#!/bin/sh
# Live test of `tilrd update` on a real machine, from the real CDN.
#
#   scripts/update-test.sh <ssh-host> <old-version>
#   scripts/update-test.sh ubu-test 0.1.0-7b5740b
#
# Installs <old-version> on the host by hand, opens panes with state in them,
# updates to the latest release and checks that nothing in the panes noticed.
# The scenarios, in order:
#
#   1. install   the old release, downloaded by hand, starts and names itself
#   2. update    run from inside a pane, with four panes open: the version
#                changes, the server keeps its pid, every pane keeps its
#                shell, its jobs, its running output and its full-screen
#                program
#   3. again     a second update finds nothing to do and reloads nothing
#   4. no cdn    an update that cannot reach the CDN fails and changes nothing
#   5. rollback  the old release put back by hand, panes open: they survive
#                the way back, and the update forward again
#   6. stopped   an update with the server stopped; the next start is the new
#                version
#
# The host needs ssh access, curl and sha256sum. The test keeps to itself: its
# server lives in ~/tilr-update-test on the host and listens on its own port,
# so an installed tilr is left alone. Prints one line per check, `ok` or
# `not ok`, and exits with 1 if any failed.
#
#   TILR_CDN         where releases are fetched from (default https://cdn.tilr.dev)
#   TILR_TEST        the test client (default ../test-client/target/debug/tilr-test)
#   TILR_TEST_PORT   port of the test server on the host (default 7899)
#   TILR_LOCAL_PORT  local end of the tunnel to it (default 7893)
#   TILR_KEEP        set to 1 to leave the server and its directory on the host
set -u
cd "$(dirname "$0")/.."

[ "$#" -eq 2 ] || { sed -n '2,5p' "$0" >&2; exit 2; }
host=$1
old=$2
cdn=${TILR_CDN:-https://cdn.tilr.dev}
client=${TILR_TEST:-../test-client/target/debug/tilr-test}
port=${TILR_TEST_PORT:-7899}
local_port=${TILR_LOCAL_PORT:-7893}
dir=tilr-update-test
socket=${TMPDIR:-/tmp}/tilr-update-test.$$

[ -x "$client" ] || { echo "no test client at $client (cargo build in test-client)" >&2; exit 2; }

failed=0
ok() { echo "ok - $1"; }
not_ok() { echo "not ok - $1${2:+: $2}"; failed=$((failed + 1)); }
# check NAME GOT WANT
check() { if [ "$2" = "$3" ]; then ok "$1"; else not_ok "$1" "got '$2', want '$3'"; fi; }

r() { ssh -o BatchMode=yes "$host" "$@"; }
t() { "$client" --url "ws://127.0.0.1:$local_port" "$@"; }

# Wait up to 30 seconds for a command on the host to succeed.
wait_remote() {
    tries=0
    until r "$1" >/dev/null 2>&1; do
        tries=$((tries + 1))
        [ "$tries" -lt 60 ] || return 1
        sleep 0.5
    done
}

# Type a line into a pane.
type_line() { t send "$1" "$2\\r"; }

# What the shell of a pane says about itself: `mark=<MARK> pid=<pid> jobs=<n>`.
nonce=0
shell_state() {
    nonce=$((nonce + 1))
    type_line "$1" "echo \"id=$nonce mark=\$MARK pid=\$\$ jobs=\$(jobs | grep -c sleep)\""
    sleep 1
    t dump "$1" | sed -n "s/^id=$nonce //p" | tail -1
}

# Put a release where the server runs from, the way it is done by hand:
# download, check, rename into place. A release from before the rename is
# only under the name tilr-server.
install_by_hand() {
    r "cd $dir && for n in tilrd tilr-server; do curl -fsSL -o staged \"$cdn/releases/$1/\$n-$platform\" 2>/dev/null && break; done &&
        [ \"\$(sha256sum staged | cut -d' ' -f1)\" = \"\$(curl -fsSL \"$cdn/releases/$1/\$n-$platform.sha256\" | cut -d' ' -f1)\" ] &&
        chmod 755 staged && mv staged tilrd"
}

start_server() {
    r "cd $dir && (TILR_RUNTIME_DIR=\$PWD/runtime setsid ./tilrd --listen 127.0.0.1:$port >>log 2>&1 </dev/null &)
        sleep 1; pgrep -f '^./tilrd --listen 127.0.0.1:$port' | head -1 >pid; test -s pid"
}

stop_server() { r "kill \$(cat $dir/pid) 2>/dev/null; sleep 1"; }
version() { r "$dir/tilrd --version" | cut -d' ' -f2; }
server_pid() { r "pid=\$(cat $dir/pid); kill -0 \$pid 2>/dev/null && echo \$pid"; }
reloads() { r "grep -c 'resumed after reload with' $dir/log"; }
pane_ids() { t list | cut -f1 | tr '\n' ' '; }
# Ticks written by pane 2: any number missing, and the longest pause.
ticks() {
    r "awk 'NR > 1 && \$2 != prev + 1 { missing++ }
            NR > 1 && \$3 - at > gap { gap = \$3 - at }
            { prev = \$2; at = \$3 }
            END { printf \"missing=%d slow=%d\", missing, (gap > 1) }' $dir/ticks"
}

cleanup() {
    ssh -S "$socket" -O exit "$host" 2>/dev/null
    [ "${TILR_KEEP:-}" = 1 ] || r "kill \$(cat $dir/pid 2>/dev/null) 2>/dev/null; rm -rf $dir"
}
trap cleanup EXIT

platform="linux-$(r uname -m)"
latest=$(curl -fsSL "$cdn/releases/$platform/latest") || { echo "no release for $platform at $cdn" >&2; exit 2; }
[ "$latest" != "$old" ] || { echo "$old is the latest release: nothing to update to" >&2; exit 2; }
echo "# $host ($platform): $old -> $latest, from $cdn"

r "kill \$(cat $dir/pid 2>/dev/null) 2>/dev/null; rm -rf $dir; mkdir $dir"
ssh -o BatchMode=yes -o ExitOnForwardFailure=yes -M -S "$socket" -fN \
    -L "127.0.0.1:$local_port:127.0.0.1:$port" "$host" || exit 2

echo "# 1. install"
install_by_hand "$old" && ok "the old release downloads and matches its checksum" ||
    not_ok "the old release downloads and matches its checksum"
start_server && ok "the old release starts" || { not_ok "the old release starts"; exit 1; }
check "it names itself $old" "$(version)" "$old"
pid=$(server_pid)

echo "# 2. update, from inside a pane"
p1=$(t new --detach)
p2=$(t new --detach)
p3=$(t new --detach)
p4=$(t new --detach -- sleep 100000)
panes="$p1 $p2 $p3 $p4 "
type_line "$p1" "MARK=survived; sleep 9999 &"
type_line "$p2" "i=0; while true; do i=\$((i+1)); echo \"tick \$i \$(date +%s.%N)\" | tee -a ~/$dir/ticks; sleep 0.2; done"
type_line "$p3" "less /etc/services"
sleep 2
before=$(shell_state "$p1")
check "pane 1 has its mark and a job before the update" "${before#* pid=*jobs=}" "1"
shell_pid=$(echo "$before" | sed 's/.*pid=\([0-9]*\).*/\1/')

type_line "$p1" "TILR_CDN=$cdn ~/$dir/tilrd update"
wait_remote "test \"\$(grep -c 'resumed after reload with' $dir/log)\" -ge 1" ||
    not_ok "the server reloads after the update"
sleep 2
check "the version is the latest release" "$(version)" "$latest"
check "the server keeps its pid" "$(server_pid)" "$pid"
check "it reloaded once, with all four panes" "$(r "grep -c 'resumed after reload with 4 pane' $dir/log")" "1"
check "the panes are the same" "$(pane_ids)" "$panes"
check "pane 1 keeps its shell, its variable and its job" "$(shell_state "$p1")" "mark=survived pid=$shell_pid jobs=1"
check "pane 2 printed through the update without a pause" "$(ticks)" "missing=0 slow=0"
check "pane 3 still shows its full-screen program" "$(t dump "$p3" | head -1)" "# Network services, Internet style"
t send "$p3" q
type_line "$p3" "echo back-in-the-shell"
sleep 1
t dump "$p3" | grep -q '^back-in-the-shell' && ok "pane 3 returns to its shell" || not_ok "pane 3 returns to its shell"
check "pane 4 still runs its command" "$(r "pgrep -fc '^sleep 100000'")" "1"

echo "# 3. update again"
again=$(r "$dir/tilrd update --cdn $cdn")
case "$again" in
    *"is the latest release"*) ok "a second update finds nothing to do" ;;
    *) not_ok "a second update finds nothing to do" "$again" ;;
esac
sleep 3
check "it reloads nothing" "$(reloads)" "1"

echo "# 4. update with the CDN out of reach"
sum=$(r "sha256sum $dir/tilrd")
r "$dir/tilrd update --cdn http://127.0.0.1:9" >/dev/null 2>&1 &&
    not_ok "the update fails" || ok "the update fails"
check "the executable is untouched" "$(r "sha256sum $dir/tilrd")" "$sum"
check "nothing is left next to it" "$(r "ls $dir | grep -c '\.new\.'")" "0"
check "the server carries on" "$(pane_ids)" "$panes"

echo "# 5. rollback by hand, then update again"
install_by_hand "$old" || not_ok "the old release is put back"
wait_remote "test \"\$(grep -c 'resumed after reload with' $dir/log)\" -ge 2" ||
    not_ok "the server reloads into the old release"
sleep 2
check "the version is the old release" "$(version)" "$old"
check "pane 1 survives the way back" "$(shell_state "$p1")" "mark=survived pid=$shell_pid jobs=1"
r "$dir/tilrd update --cdn $cdn" >/dev/null
wait_remote "test \"\$(grep -c 'resumed after reload with' $dir/log)\" -ge 3" ||
    not_ok "the server reloads into the latest release"
sleep 2
check "the version is the latest release again" "$(version)" "$latest"
check "the server still has its pid" "$(server_pid)" "$pid"
check "the panes are the same" "$(pane_ids)" "$panes"
check "pane 1 survives the way forward" "$(shell_state "$p1")" "mark=survived pid=$shell_pid jobs=1"
check "pane 2 printed through all three reloads without a pause" "$(ticks)" "missing=0 slow=0"
check "nothing was refused or lost" "$(r "grep -cE 'not reloading|WARN|ERROR' $dir/log")" "0"

echo "# 6. update with the server stopped"
stop_server
install_by_hand "$old" || not_ok "the old release is put back"
r "$dir/tilrd update --cdn $cdn" >/dev/null && ok "the update runs without a server" ||
    not_ok "the update runs without a server"
start_server && ok "the server starts" || not_ok "the server starts"
check "it is the latest release" "$(version)" "$latest"
p=$(t new --detach) && ok "it opens a pane" || not_ok "it opens a pane"

if [ "$failed" -eq 0 ]; then
    echo "# all checks passed"
else
    echo "# $failed check(s) failed"
    exit 1
fi
