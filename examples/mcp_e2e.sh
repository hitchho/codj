#!/bin/bash
# Drives `codj mcp` over stdin/stdout JSON-RPC against examples/fake_codjd.rs on an ISOLATED bus
# and checks the set tools: initialize (clientInfo "tester"), tools/list, set_source, set_check,
# set_write with a stale and a fresh hash, capture and record, and set_arm refused without --live;
# the patch tools too: node catalog, check, write with base_hash, import, state, actions, and
# patch_load / patch_unload / patch_live refused without --live (and an edit of a live slot).
#
#   cargo build --examples && cargo build
#   dbus-run-session -- examples/mcp_e2e.sh
set -u
[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || { echo "run under dbus-run-session"; exit 2; }
case "$DBUS_SESSION_BUS_ADDRESS" in *"/run/user/"*) echo "that looks like the real session bus"; exit 2;; esac
W=${W:-/tmp/codj-mcp-e2e}
BIN=${BIN:-$(dirname "$0")/../target/debug}
mkdir -p "$W"; rm -f "$W"/calls.log "$W"/*.err
export HOME=$W/home XDG_CACHE_HOME=$W/cache XDG_CONFIG_HOME=$W/config
mkdir -p "$HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME"
export CODJ_FAKE_ISOLATED=1 CODJ_FAKE_LOG=$W/calls.log
"$BIN/examples/fake_codjd" 2>"$W/fake.err" & FAKE=$!
sleep 1
FAILS=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; FAILS=$((FAILS+1)); }
check() { if eval "$2"; then ok "$1"; else bad "$1: $2"; fi; }

ID=0
# rpc <method> <params-json>: sends a request, waits for its response, leaves it in $R
rpc() {
  ID=$((ID+1))
  printf '{"jsonrpc":"2.0","id":%d,"method":"%s","params":%s}\n' "$ID" "$1" "$2" >&"${MCP[1]}"
  while IFS= read -r -t 10 R <&"${MCP[0]}"; do
    [ "$(jq -r '.id // empty' <<<"$R")" = "$ID" ] && return 0
  done
  R='{"error":"timeout"}'; return 1
}
# tool <name> <arguments-json>: tools/call; $R is the response, $T its text, $E "true" on isError
tool() {
  rpc tools/call "$(jq -nc --arg n "$1" --argjson a "$2" '{name:$n,arguments:$a}')"
  T=$(jq -r '.result.content[0].text // .error.message // empty' <<<"$R")
  E=$(jq -r '.result.isError // false' <<<"$R")
}
start() { # start [--live]
  coproc MCP { "$BIN/codj" mcp "$@" 2>>"$W/mcp.err"; }
  rpc initialize '{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"tester","version":"1"}}'
  printf '{"jsonrpc":"2.0","method":"notifications/initialized"}\n' >&"${MCP[1]}"
}
stop() { exec {MCP[1]}>&-; wait "$MCP_PID" 2>/dev/null; }

echo "== safe mode"
start
check "initialize answers" '[ -n "$(jq -r .result.serverInfo.name <<<"$R")" ]'
rpc tools/list '{}'
NAMES=$(jq -r '.result.tools[].name' <<<"$R")
for t in sets_list set_source set_check set_write set_use set_cue set_loop_capture set_grid_capture set_record set_arm set_disarm set_auto set_preview set_load set_language_help; do
  check "tools/list has $t" 'grep -qx "$t" <<<"$NAMES"'
done
check "set_write documents base_hash" 'jq -e ".result.tools[] | select(.name==\"set_write\") | .description | test(\"base_hash\")" <<<"$R" >/dev/null'

tool sets_list '{}'
check "sets_list lists demo" 'jq -e "[.sets[].name] | index(\"demo\") != null" <<<"$T" >/dev/null'
tool set_source '{"name":"demo"}'
H1=$(jq -r .hash <<<"$T"); TEXT=$(jq -r .text <<<"$T")
check "set_source returns text and hash" '[ -n "$H1" ] && grep -q "track warm" <<<"$TEXT"'
tool set_check "$(jq -nc --arg t "$TEXT" '{text:$t}')"
check "set_check: valid text" '[ "$(jq -r .valid <<<"$T")" = true ]'
tool set_check "$(jq -nc --arg t "$TEXT
  BAD" '{text:$t}')"
check "set_check: error with file:line:col" '[ "$(jq -r .valid <<<"$T")" = false ] && jq -r ".errors[0]" <<<"$T" | grep -q "demo.codj:[0-9]*:[0-9]*:"'

NEW="$TEXT
  cue outro bar 129"
tool set_write "$(jq -nc --arg t "$NEW" '{name:"demo",text:$t,base_hash:"deadbeef"}')"
check "set_write with a stale hash is refused" '[ "$E" = true ] && grep -q "set changed" <<<"$T" && grep -q set_source <<<"$T"'
tool set_write "$(jq -nc --arg t "$TEXT
  BAD" --arg h "$H1" '{name:"demo",text:$t,base_hash:$h}')"
check "set_write with errors is refused" '[ "$E" = true ] && grep -q "syntax error" <<<"$T"'
tool set_write "$(jq -nc --arg t "$NEW" --arg h "$H1" '{name:"demo",text:$t,base_hash:$h}')"
H2=$(jq -r .hash <<<"$T")
check "set_write with the fresh hash works" '[ "$E" = false ] && [ -n "$H2" ] && [ "$H2" != "$H1" ]'
tool set_write "$(jq -nc --arg t "$NEW" --arg h "$H1" '{name:"demo",text:$t,base_hash:$h}')"
check "the old hash is stale after a write" '[ "$E" = true ] && grep -q "set changed" <<<"$T"'
tool set_write '{"name":"demo","text":"x"}'
check "set_write without base_hash is rejected" '[ "$E" = true ] || jq -e .error <<<"$R" >/dev/null'

tool set_cue '{"deck":0,"name":"drop"}'
check "set_cue without an active set fails" '[ "$E" = true ] && grep -q "no active set" <<<"$T"'
tool set_use '{"name":"demo"}'
check "set_use works" '[ "$E" = false ]'
tool set_cue '{"deck":0,"name":"drop"}'
check "set_cue on an empty deck says so" '[ "$E" = true ] && grep -q "nothing loaded" <<<"$T"'
tool set_cue '{"deck":0,"name":"not a name"}'
check "set_cue rejects a bad name" '[ "$E" = true ] && grep -q "bad name" <<<"$T"'
tool set_record '{"on":true}'
check "set_record start" '[ "$E" = false ]'
sleep 0.3
tool set_record '{"on":false}'
check "set_record stop returns the summary" '[ "$E" = false ] && jq -r .detail <<<"$T" | grep -q "mix 2: aft -> smk, 16 bars, 9 items"'
tool set_language_help '{}'
check "set_language_help is a cheat sheet" 'grep -q "swap" <<<"$T"'

echo "== patch tools (safe mode)"
for t in patch_list patch_source patch_check patch_write patch_import patch_action patch_state patch_node_types patch_load patch_unload patch_live; do
  check "tools/list has $t" 'grep -qx "$t" <<<"$NAMES"'
done
rpc tools/list '{}'
check "patch_write documents base_hash" 'jq -e ".result.tools[] | select(.name==\"patch_write\") | .description | test(\"base_hash\")" <<<"$R" >/dev/null'
check "patch_check documents the workflow" 'jq -e ".result.tools[] | select(.name==\"patch_list\") | .description | test(\"patch_node_types.*patch_check.*patch_write.*patch_load.*patch_action.*patch_live\")" <<<"$R" >/dev/null'
tool patch_node_types '{}'
check "patch_node_types has the catalog" '[ "$E" = false ] && jq -e "[.nodes[].type] | (index(\"GateSeq\") != null) and (index(\"AudioOut\") != null)" <<<"$T" >/dev/null && jq -e ".nodes[] | select(.type==\"Filter\") | .inputs.cutoff == 1" <<<"$T" >/dev/null'
tool patch_list '{}'
check "patch_list lists acid" 'jq -e "[.patches[].name] | index(\"acid\") != null" <<<"$T" >/dev/null'
tool patch_source '{"name":"acid"}'
PH1=$(jq -r .hash <<<"$T"); PJSON=$(jq -r .json <<<"$T")
check "patch_source returns json and hash" '[ -n "$PH1" ] && jq -e .nodes <<<"$PJSON" >/dev/null'
tool patch_source '{"name":"../etc/passwd"}'
check "patch_source rejects a path-like name" '[ "$E" = true ] && grep -q "bad patch name" <<<"$T"'
tool patch_check "$(jq -nc --arg j "$PJSON" '{json:$j}')"
check "patch_check: valid patch" '[ "$(jq -r .valid <<<"$T")" = true ]'
NOOUT=$(jq -c 'del(.nodes[] | select(.type=="AudioOut"))' <<<"$PJSON")
tool patch_check "$(jq -nc --arg j "$NOOUT" '{json:$j}')"
check "patch_check: no AudioOut is an error" '[ "$(jq -r .valid <<<"$T")" = false ] && jq -r ".errors[0]" <<<"$T" | grep -q AudioOut'
BADTYPE=$(jq -c '.nodes["3"].type = "Zonk"' <<<"$PJSON")
tool patch_check "$(jq -nc --arg j "$BADTYPE" '{json:$j}')"
check "patch_check: error names the node" 'jq -r ".errors[0]" <<<"$T" | grep -q "^3: unknown type Zonk"'
tool patch_check '{"json":"{not json"}'
check "patch_check rejects non-JSON before the daemon" '[ "$E" = true ] && grep -q "not valid JSON" <<<"$T"'
NEWP=$(jq -c '.title = "acid two"' <<<"$PJSON")
tool patch_write "$(jq -nc --arg j "$NEWP" '{name:"acid",json:$j,base_hash:"deadbeef"}')"
check "patch_write with a stale hash is refused" '[ "$E" = true ] && grep -q "changed" <<<"$T" && grep -q patch_source <<<"$T"'
tool patch_write "$(jq -nc --arg j "$NOOUT" --arg h "$PH1" '{name:"acid",json:$j,base_hash:$h}')"
check "patch_write with errors is refused" '[ "$E" = true ] && grep -q "refused" <<<"$T"'
tool patch_write "$(jq -nc --arg j "$NEWP" --arg h "$PH1" '{name:"acid",json:$j,base_hash:$h}')"
PH2=$(jq -r .hash <<<"$T")
check "patch_write with the fresh hash works" '[ "$E" = false ] && [ -n "$PH2" ] && [ "$PH2" != "$PH1" ]'
tool patch_write "$(jq -nc --arg j "$NEWP" --arg h "$PH1" '{name:"acid",json:$j,base_hash:$h}')"
check "the old patch hash is stale after a write" '[ "$E" = true ] && grep -q "changed" <<<"$T"'
tool patch_write '{"name":"acid","json":"{}"}'
check "patch_write without base_hash is rejected" '[ "$E" = true ] || jq -e .error <<<"$R" >/dev/null'
tool patch_write "$(jq -nc --arg j "$NEWP" '{name:"fresh1",json:$j,base_hash:""}')"
check "patch_write creates a new patch with an empty base_hash" '[ "$E" = false ]'
tool patch_import '{"path":"relative.ncft","name":"x"}'
check "patch_import wants an absolute .ncft path" '[ "$E" = true ] && grep -q "absolute" <<<"$T"'
tool patch_import '{"path":"/nonexistent/none.ncft","name":"lead"}'
check "patch_import passes the daemon's reason on" '[ "$E" = true ] && grep -q "cannot read" <<<"$T"'
tool patch_state '{}'
check "patch_state: four empty slots" '[ "$(jq -r ".slots | length" <<<"$T")" = 4 ] && [ "$(jq -r ".slots[0].loaded" <<<"$T")" = false ]'
tool patch_action '{"slot":0,"action":{"op":"knob","name":"cutoff","value":0.5}}'
check "patch_action on an empty slot says so" '[ "$E" = true ] && grep -q "empty" <<<"$T"'
tool patch_action '{"slot":7,"action":{"op":"knob"}}'
check "patch_action rejects a bad slot" '[ "$E" = true ] && grep -q "0..3" <<<"$T"'
tool patch_action '{"slot":0,"action":{"op":"explode"}}'
check "patch_action rejects an unknown op" '[ "$E" = true ] && grep -q "unknown op" <<<"$T"'
for call in 'patch_load {"slot":0,"name":"acid"}' 'patch_unload {"slot":0}' 'patch_live {"slot":0,"live":true}'; do
  set -- $call
  tool "$1" "$2"
  check "$1 refused without --live" '[ "$E" = true ] && grep -q -- "--live" <<<"$T"'
done

for call in 'set_arm {"mix":0}' 'set_disarm {}' 'set_auto {"on":true}' 'set_preview {"mix":0}' 'set_load {"track":0}'; do
  set -- $call
  tool "$1" "$2"
  check "$1 refused without --live" '[ "$E" = true ] && grep -q -- "--live" <<<"$T"'
done
stop

echo "== --live"
start --live
tool set_arm '{"mix":0}'
# the fake answers like the daemon: the outgoing track is not on a deck. What matters is that it got through
check "set_arm allowed with --live (reaches the daemon)" '! grep -q -- "--live" <<<"$T"'
# a slot loaded by hand (this test plays the DJ over D-Bus) and sent live: agent edits of it need --live
busctl --user call net.rotko.Codj /net/rotko/Codj net.rotko.Codj.Player PatchLoad us 1 acid >/dev/null
busctl --user call net.rotko.Codj /net/rotko/Codj net.rotko.Codj.Player PatchLive ub 1 true >/dev/null
start
tool patch_state '{}'
check "patch_state shows the loaded patch, live, with its knobs" '[ "$(jq -r ".slots[1].name" <<<"$T")" = acid ] && [ "$(jq -r ".slots[1].live" <<<"$T")" = true ] && jq -e ".slots[1].knobs | map(.name) | index(\"cutoff\") != null" <<<"$T" >/dev/null && jq -e ".slots[1].sequencers[0].queued == null" <<<"$T" >/dev/null'
tool patch_action '{"slot":1,"action":{"op":"knob","name":"cutoff","value":0.9}}'
check "patch_action on a LIVE slot is refused without --live" '[ "$E" = true ] && grep -q -- "--live" <<<"$T"'
stop
busctl --user call net.rotko.Codj /net/rotko/Codj net.rotko.Codj.Player PatchLive ub 1 false >/dev/null
start
tool patch_action '{"slot":1,"action":{"op":"knob","name":"cutoff","value":0.9}}'
check "patch_action on a cue-only slot works in safe mode" '[ "$E" = false ]'
tool patch_action '{"slot":1,"action":{"op":"cell","seq":"seq","pattern":0,"step":1,"row":4,"on":true,"velocity":0.7}}'
check "patch_action cell" '[ "$E" = false ]'
tool patch_action '{"slot":1,"action":{"op":"pattern","seq":"seq","pattern":1,"now":false}}'
check "patch_action pattern (queued)" '[ "$E" = false ]'
tool patch_state '{}'
check "patch_state: the knob moved, the pattern is queued, the slot is dirty" '[ "$(jq -r ".slots[1].knobs[] | select(.name==\"cutoff\") | .value" <<<"$T")" = 0.9 ] && [ "$(jq -r ".slots[1].sequencers[0].queued" <<<"$T")" = 1 ] && [ "$(jq -r ".slots[1].unsaved_edits" <<<"$T")" = true ]'
tool patch_action '{"slot":1,"action":"{\"op\":\"knob\",\"name\":\"nope\",\"value\":1}"}'
check "a double-encoded action is accepted; the daemon's reason comes back" '[ "$E" = true ] && grep -q "no knob nope" <<<"$T"'
stop

echo "== --live (patch)"
start --live
tool patch_load '{"slot":2,"name":"pad1"}'
check "patch_load allowed with --live" '[ "$E" = false ]'
tool patch_live '{"slot":2,"live":true}'
check "patch_live allowed with --live" '[ "$E" = false ]'
tool patch_action '{"slot":2,"action":{"op":"knob","name":"cutoff","value":0.8}}'
check "patch_action on a live slot is allowed with --live" '[ "$E" = false ]'
tool patch_unload '{"slot":2}'
check "patch_unload allowed with --live" '[ "$E" = false ]'
stop

kill $FAKE; wait $FAKE 2>/dev/null
echo "== calls received by the fake"; cat "$W/calls.log"
check "writes carry author agent:tester" 'grep -q "SetWrite demo base=$H1 author=agent:tester" "$W/calls.log"'
check "refused tools never reached the daemon" '! grep -q "^SetDisarm\|^SetAuto\|^SetPreview\|^SetLoad" "$W/calls.log" && [ "$(grep -c "^SetArm" "$W/calls.log")" = 1 ]'
check "patch writes carry author agent:tester" 'grep -q "PatchWrite acid base=$PH1 author=agent:tester" "$W/calls.log"'
check "refused patch calls never reached the daemon" '! grep -q "^PatchLoad 0\|^PatchUnload 0\|^PatchLive 0" "$W/calls.log" && [ "$(grep -c "PatchAction 1 .*\"value\":0.9" "$W/calls.log")" = 1 ]'
echo "== codj mcp stderr"; cat "$W/mcp.err"
[ "$FAILS" = 0 ] && echo "ALL OK" || { echo "$FAILS FAILED"; exit 1; }
