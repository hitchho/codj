#!/bin/bash
# A stand-in for an AI agent, for examples/dj_e2e.sh. codj runs it like any agent command:
# the prompt (preamble + request) arrives on stdin, CODJ_MCP_CONFIG names the MCP config file.
# It prints a few lines slowly and then edits the active set over D-Bus (busctl) the way the
# codj MCP tools would: SetSource, then SetWrite with the hash and author "agent:fake".
# It must run on an isolated bus (it uses $DBUS_SESSION_BUS_ADDRESS from codj).
set -u
prompt=$(cat)
echo "fake agent: got $(printf %s "$prompt" | wc -c) bytes of prompt"
echo "mcp config: ${CODJ_MCP_CONFIG:-none}  live: ${CODJ_MCP_LIVE:-?}"
echo "request: $(printf %s "$prompt" | tail -n 1)"
set_name=$(printf %s "$prompt" | sed -n 's/^Active set: \([^ ]*\).*/\1/p' | head -n 1)
[ -n "$set_name" ] || { echo "no active set in the prompt" >&2; exit 1; }
sleep 0.6
echo "reading $set_name with set_source"
src=$(busctl --user --json=short call net.rotko.Codj /net/rotko/Codj net.rotko.Codj.Player SetSource s "$set_name") || exit 1
text=$(printf %s "$src" | jq -r '.data[0]')
hash=$(printf %s "$src" | jq -r '.data[1]')
sleep 0.6
echo "this one is a little short; making the first mix longer"
text=$(printf %s "$text" | sed 's/for 16 bars/for 32 bars/')$'\n'
echo "set_check on the new text"
sleep 0.6
errs=$(busctl --user --json=short call net.rotko.Codj /net/rotko/Codj net.rotko.Codj.Player SetCheck s "$text" | jq -r '.data[0] | length')
echo "set_check: $errs error(s)"
[ "$errs" = "0" ] || exit 2
echo "set_write with base hash ${hash:0:8}"
sleep 0.4
reply=$(busctl --user --json=short call net.rotko.Codj /net/rotko/Codj net.rotko.Codj.Player SetWrite ssss "$set_name" "$text" "$hash" "agent:fake" | jq -r '.data[0]')
echo "reply: $reply"
echo "done: the first mix is now 32 bars"
