#!/bin/bash
# Runs codj against examples/fake_codjd.rs in tmux on an ISOLATED session bus and prints what
# the fake received. Terminal captures land in $W/cap-*.txt.
#
#   cargo build --examples && cargo build
#   dbus-run-session -- examples/dj_e2e.sh
#
# tmux does not forward the kitty keyboard protocol, so this exercises the no-release fallback.
set -u
[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || { echo "run under dbus-run-session"; exit 2; }
case "$DBUS_SESSION_BUS_ADDRESS" in *"/run/user/"*) echo "that looks like the real session bus"; exit 2;; esac
W=${W:-/tmp/codj-dj-e2e}
BIN=${BIN:-$(dirname "$0")/../target/debug}
export HOME=$W/home XDG_CACHE_HOME=$W/cache XDG_CONFIG_HOME=$W/config
mkdir -p "$HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME"
rm -f "$W"/calls.log "$W"/cap-*
export CODJ_FAKE_ISOLATED=1 CODJ_FAKE_LOG=$W/calls.log
"$BIN/examples/fake_codjd" 2>"$W/fake.err" & FAKE=$!
sleep 1
T="tmux -L codjdje2e"
$T new-session -d -x 140 -y 40 -s dj "$BIN/codj 2>$W/codj.err; sleep 30"
cap() { $T capture-pane -p -t dj > "$W/cap-$1.txt"; }
keys() { for k in "$@"; do $T send-keys -t dj -- "$k"; sleep 0.12; done; }
sleep 2
cap normal
keys M; sleep 0.5; cap entered
keys Enter Down '>'; sleep 1          # load A and B
keys x '.'; sleep 1.5; cap playing
keys c 1 v; sleep 1; cap sync_hotcue_loop
keys M-q M-q M-r g t F1 F5 w; sleep 0.5; cap mixer
keys z; sleep 0.3; keys z '!'
keys Tab p a l; sleep 0.3; cap search
keys Enter '>'; sleep 0.5
keys ':' q; cap command
keys Enter; sleep 0.5; cap back_to_normal
keys M; sleep 0.3; keys Escape; sleep 0.5; cap esc_exit
$T kill-server
kill $FAKE
echo "== calls received by the fake"; cat "$W/calls.log"
echo "== codj stderr"; cat "$W/codj.err"
