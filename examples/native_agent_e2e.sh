#!/bin/bash
# codj's own agent loop against examples/fake_chat_api.py (an OpenAI-compatible API on localhost)
# and examples/fake_codjd.rs, in tmux on an ISOLATED session bus. Spends no API credits.
#
#   cargo build && cargo build --examples
#   dbus-run-session -- examples/native_agent_e2e.sh
set -u
[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || { echo "run under dbus-run-session"; exit 2; }
case "$DBUS_SESSION_BUS_ADDRESS" in *"/run/user/"*) echo "that looks like the real session bus"; exit 2;; esac
W=${W:-/tmp/codj-native-e2e}
BIN=${BIN:-$(dirname "$0")/../target/debug}
HERE=$(cd "$(dirname "$0")" && pwd)
rm -rf "$W"; mkdir -p "$W"
export HOME=$W/home XDG_CACHE_HOME=$W/cache XDG_CONFIG_HOME=$W/config XDG_STATE_HOME=$W/state XDG_DATA_HOME=$W/data
mkdir -p "$HOME/.config/codj" "$W/bin"
# a misread config must never reach a real agent CLI: `claude` here only fails
printf '#!/bin/sh\necho "the test reached claude: the config was not read" >&2\nexit 99\n' > "$W/bin/claude"
chmod +x "$W/bin/claude"
export PATH="$W/bin:$PATH"
export CODJ_FAKE_ISOLATED=1 CODJ_FAKE_LOG=$W/calls.log FAKE_KEY=sk-test-123
PORT=${PORT:-18765}
printf '[agent]\nbase_url = "http://127.0.0.1:%s/v1"\nmodel = "fake-model"\napi_key_env = "FAKE_KEY"\n' "$PORT" > "$HOME/.config/codj/config.toml"
python3 -I "$HERE/fake_chat_api.py" "$PORT" "$W/api.log" & API=$!
"$BIN/examples/fake_codjd" 2>"$W/fake.err" & FAKE=$!
sleep 1
T="tmux -L ${TMUX_SOCK:-codjnative}"
S=native
$T new-session -d -x 140 -y 40 -s $S "$BIN/codj 2>$W/codj.err; sleep 30"
sleep 2
keys() { $T send-keys -t $S "$@"; sleep 0.3; }
cmd() { keys ':'; $T send-keys -t $S -l -- "$1"; sleep 0.15; $T send-keys -t $S Enter; sleep 0.6; }
cap() { $T capture-pane -p -t $S > "$W/cap-$1.txt"; }
keys M; sleep 0.5
cmd 'set demo'
cmd 'ai is this set ok'
sleep 4; cap done
keys Escape; sleep 0.3
cmd 'ai and the second mix'
sleep 4; cap followup
$T kill-server; kill $API $FAKE 2>/dev/null
F=0
chk() { if grep -q -- "$3" "$W/$2"; then echo "ok   $1"; else echo "FAIL $1 ($2: $3)"; F=1; fi; }
chk "the pane names the endpoint" cap-done.txt '\$ api fake-model'
chk "the agent's text" cap-done.txt 'Reading the set first.'
chk "its tool call" cap-done.txt '→ set_source name=demo'
chk "the tool's result" cap-done.txt '  ✓ '
chk "the answer after the tool" cap-done.txt 'Nothing to change.'
chk "the summary" cap-done.txt '-- done: 2 turns'
chk "codj's tools were offered" api.log '"name": "set_write"'
chk "the key went as a bearer token" api.log 'Bearer sk-test-123'
chk "the tool result went back under its call id" api.log '"tool_call_id": "call_1"'
chk "the follow-up carries the conversation" api.log 'is this set ok.*and the second mix'
[ "$F" = 0 ] && echo "NATIVE AGENT OK" || { echo "NATIVE AGENT FAILED"; cat "$W/codj.err"; exit 1; }
