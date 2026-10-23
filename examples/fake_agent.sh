#!/bin/bash
# A stand-in for an AI agent, for examples/dj_e2e.sh. codj runs it like any agent command:
# the prompt (preamble + request) arrives on stdin, CODJ_MCP_CONFIG names the MCP config file.
# It prints Claude Code's stream-json events slowly and then edits the active set over D-Bus (busctl) the way the
# codj MCP tools would: SetSource, then SetWrite with the hash and author "agent:fake".
# It must run on an isolated bus (it uses $DBUS_SESSION_BUS_ADDRESS from codj).
set -u
prompt=$(cat)
# events in Claude Code's stream-json format, as `claude -p --output-format stream-json` prints them
say()    { jq -nc --arg t "$1" '{type:"assistant",message:{content:[{type:"text",text:$t}]}}'; }
tool()   { jq -nc --arg n "mcp__codj__$1" --argjson i "$2" '{type:"assistant",message:{content:[{type:"tool_use",name:$n,input:$i}]}}'; }
result() { jq -nc --arg t "$1" --argjson e "${2:-false}" '{type:"user",message:{content:[{type:"tool_result",content:$t,is_error:$e}]}}'; }
start=$(date +%s%N)
jq -nc '{type:"system",subtype:"init",model:"fake-agent",tools:["set_source","set_check","set_write"]}'
say "got $(printf %s "$prompt" | wc -c) bytes of prompt; mcp config: ${CODJ_MCP_CONFIG:-none}, live: ${CODJ_MCP_LIVE:-?}"
set_name=$(printf %s "$prompt" | sed -n 's/^Active set: \([^ ]*\).*/\1/p' | head -n 1)
[ -n "$set_name" ] || { say "no active set in the prompt"; exit 1; }
sleep 0.6
tool set_source "$(jq -nc --arg n "$set_name" '{name:$n}')"
src=$(busctl --user --json=short call net.rotko.Codj /net/rotko/Codj net.rotko.Codj.Player SetSource s "$set_name") || exit 1
text=$(printf %s "$src" | jq -r '.data[0]')
hash=$(printf %s "$src" | jq -r '.data[1]')
result "$(printf %s "$text" | wc -l) lines, hash ${hash:0:8}"
sleep 0.6
say "This one is a little short; making the first mix longer."
text=$(printf %s "$text" | sed 's/for 16 bars/for 32 bars/')$'\n'
tool set_check "$(jq -nc --arg t "$text" '{text:$t}')"
sleep 0.6
errs=$(busctl --user --json=short call net.rotko.Codj /net/rotko/Codj net.rotko.Codj.Player SetCheck s "$text" | jq -r '.data[0] | length')
result "$errs error(s)"
[ "$errs" = "0" ] || exit 2
tool set_write "$(jq -nc --arg n "$set_name" --arg t "$text" --arg h "$hash" '{name:$n,text:$t,base_hash:$h}')"
sleep 0.4
reply=$(busctl --user --json=short call net.rotko.Codj /net/rotko/Codj net.rotko.Codj.Player SetWrite ssss "$set_name" "$text" "$hash" "agent:fake" | jq -r '.data[0]')
result "$reply"
say "Done: the first mix is now 32 bars."
ms=$(( ($(date +%s%N) - start) / 1000000 ))
jq -nc --argjson d "$ms" '{type:"result",subtype:"success",num_turns:4,duration_ms:$d,total_cost_usd:0,is_error:false}'
