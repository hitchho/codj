#!/bin/bash
# Runs codj against examples/fake_codjd.rs in tmux on an ISOLATED session bus and prints what
# the fake received. Terminal captures land in $W/cap-*.txt.
#
#   cargo build --examples && cargo build
#   dbus-run-session -- examples/dj_e2e.sh
#
# Phase 2 drives the set runtime (:set, F6/F7/F8, :preview, :edit with a fake $EDITOR).
# Phase 3 drives capture and recording (:cue, :loopsave, :gridsave, :rec, F9).
# Phase 4 drives the editor pane (F10: insert, error marker, :w, write conflict) and the AI
# prompt box (:ai with examples/fake_agent.sh as the agent: streamed output, diff, undo).
# Phase 5 drives the control hints: f, a label, ] [ \ ; edits, the mouse wheel (SGR mouse
# sequences sent as literal keys), the beat-jump keys e r u i, Esc.
# Needs busctl and jq (the fake agent and the conflict step talk to the fake codjd with them).
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
S=dj
cap() { $T capture-pane -p -t $S > "$W/cap-$1.txt"; }
keys() { for k in "$@"; do $T send-keys -t $S -- "$k"; sleep 0.12; done; }
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
wait $FAKE 2>/dev/null
echo "== calls received by the fake"; cat "$W/calls.log"
echo "== codj stderr"; cat "$W/codj.err"

# --- phase 2: the set runtime (fresh fake: both decks empty, no set active)
export CODJ_FAKE_LOG=$W/calls-set.log
rm -f "$W"/calls-set.log "$W"/edit.log
cat > "$W/fake-editor.sh" <<EOF2
#!/bin/sh
echo "editor got: \$*" >> $W/edit.log
sleep 0.3
EOF2
chmod +x "$W/fake-editor.sh"
"$BIN/examples/fake_codjd" 2>"$W/fake2.err" & FAKE=$!
sleep 1
S=djset
$T new-session -d -x 140 -y 40 -s $S "EDITOR=$W/fake-editor.sh $BIN/codj 2>$W/codj-set.err; sleep 30"
sleep 2
# a ':' command followed by Enter
cmd() { keys ':'; $T send-keys -t $S -l -- "$1"; sleep 0.15; $T send-keys -t $S Enter; sleep 0.5; }
keys M; sleep 0.5
cmd arm\ 1; cap set_none_active           # no set yet: a note, no call
cmd set; cap set_list
cmd set\ broken; cap set_broken
cmd errors; cap set_errors                 # overlay
keys Escape; sleep 0.3
cmd set\ demo; sleep 0.5
cmd arm\ 1; cap set_arm_unloaded          # outgoing track not on a deck: daemon says load first
keys Enter x; sleep 0.5                    # Aftertekno onto A, play
keys F6; sleep 1; cap set_armed            # waiting for the mix start
sleep 6; cap set_running                   # started, first items consumed
cmd auto\ on; cap set_auto
keys F8; sleep 0.8; cap set_disarmed
keys F6; sleep 0.8; keys F7; sleep 0.8; cap set_f6_f7
cmd preview\ 2\ 4; cap set_preview
cmd load\ 3; cap set_load
cmd edit; sleep 1.2; cap set_after_edit    # the TUI came back after the editor
keys Escape; sleep 0.5
$T kill-server
kill $FAKE
echo "== set calls received by the fake"; cat "$W/calls-set.log"
echo "== editor"; cat "$W/edit.log"
echo "== codj stderr (set phase)"; cat "$W/codj-set.err"

# --- phase 3: capture and recording (fresh fake again)
export CODJ_FAKE_LOG=$W/calls-cap.log
rm -f "$W"/calls-cap.log
"$BIN/examples/fake_codjd" 2>"$W/fake3.err" & FAKE=$!
sleep 1
S=djcap
$T new-session -d -x 140 -y 40 -s $S "$BIN/codj 2>$W/codj-cap.err; sleep 30"
sleep 2
keys M; sleep 0.5
cmd cue\ drop; cap cap_no_set             # no active set: a note, no call
cmd set\ demo
cmd cue\ drop; cap cap_no_track           # nothing loaded: the deck rule says so
keys Enter x; sleep 0.8                    # Aftertekno onto A, play
cmd cue\ drop; cap cap_cue_a              # A is the only deck playing: SetCue 0
keys Down '>'; sleep 0.8                   # Smokin onto B (not playing)
cmd cue\ hit\ b; cap cap_cue_b           # explicit deck: SetCue 1
cmd cue\ mid\ c; cap cap_cue_bad_deck    # usage note, no call
cmd loopsave\ brk; cap cap_loop_none      # no active loop: daemon's reply as an error
cmd gridsave\ b; cap cap_grid
keys F9; sleep 1.6; cap cap_recording      # strip shows the red REC badge, counting
keys F9; sleep 0.8; cap cap_recorded       # "recorded mix 2: aft → smk, 16 bars, 9 items"
cmd rec\ on; sleep 1; cap cap_rec_on
cmd rec\ off; sleep 0.5
cmd rec; sleep 0.5; cmd rec; sleep 0.5     # toggle on, toggle off
keys Escape; sleep 0.5
$T kill-server
kill $FAKE
echo "== capture calls received by the fake"; cat "$W/calls-cap.log"
echo "== codj stderr (capture phase)"; cat "$W/codj-cap.err"
# --- phase 4: editor pane and AI prompt box
export CODJ_FAKE_LOG=$W/calls-edit.log
rm -f "$W"/calls-edit.log
AGENT=$(cd "$(dirname "$0")" && pwd)/fake_agent.sh
mkdir -p "$HOME/.config/codj"
printf '[agent]\ncommand = "%s"\n' "$AGENT" > "$HOME/.config/codj/config.toml"
"$BIN/examples/fake_codjd" 2>"$W/fake4.err" & FAKE=$!
sleep 1
S=djedit
$T new-session -d -x 140 -y 40 -s $S "$BIN/codj 2>$W/codj-edit.err; sleep 30"
sleep 2
# literal text (no key names)
typ() { $T send-keys -t $S -l -- "$1"; sleep 0.3; }
bus() { busctl --user --json=short call net.rotko.Codj /net/rotko/Codj net.rotko.Codj.Player "$@"; }
keys M; sleep 0.5
cmd set\ demo
keys F10; sleep 1; cap edit_open
keys G i; typ 'mix peak -> warm ERROR here'; sleep 1; cap edit_insert_error   # still in insert mode
keys Escape; sleep 0.9; cap edit_error_marker
keys ':'; typ w; keys Enter; sleep 0.8; cap edit_write_refused               # SetWrite refuses: errors
keys d d o; typ '  bar 4 A loop 2 beats'; keys Escape; sleep 0.9; cap edit_fixed
keys ':'; typ w; keys Enter; sleep 0.8; cap edit_written
# someone else saves, then we save on a stale hash
hash=$(bus SetSource s demo | jq -r '.data[1]')
bus SetWrite ssss demo "# demo set (saved elsewhere)
track warm \"Aftertekno\"
" "$hash" agent:other >/dev/null
keys i; typ '# '; keys Escape
keys ':'; typ w; keys Enter; sleep 1; cap edit_conflict
keys d; sleep 0.4; cap edit_conflict_diff
keys o; sleep 1; cap edit_overwritten
keys Escape; sleep 0.5; cap edit_closed
# the prompt box
cmd ai\ make\ it\ longer
sleep 1.3; cap ai_running
sleep 3.5; cap ai_review
keys n; sleep 1.2; cap ai_undone
keys Escape; sleep 0.5; cap ai_closed
keys z x; sleep 0.5                    # deck keys are live again
$T kill-server
kill $FAKE
echo "== editor/prompt calls received by the fake"; cat "$W/calls-edit.log"
echo "== codj stderr (editor phase)"; cat "$W/codj-edit.err"
# --- phase 5: control hints, selection editing, mouse wheel
export CODJ_FAKE_LOG=$W/calls-hints.log
rm -f "$W"/calls-hints.log
rm -f "$HOME/.config/codj/config.toml"
"$BIN/examples/fake_codjd" 2>"$W/fake5.err" & FAKE=$!
sleep 1
S=djhint
$T new-session -d -x 140 -y 40 -s $S "$BIN/codj 2>$W/codj-hints.err; sleep 30"
sleep 2
# a mouse wheel detent as an SGR mouse report: button 64 = up, 65 = down
wheel() { for i in $(seq "$2"); do $T send-keys -t $S -l -- "$(printf '\033[<%s;40;12M' "$1")"; sleep 0.15; done; }
keys M; sleep 0.5
keys Enter Down '>'; sleep 1                   # decks loaded
keys e r u i; sleep 0.5; cap hint_jump_keys      # DjBeatJump A -4/+4, B -4/+4
keys f; sleep 0.4; cap hints_shown              # labels on every control
keys a; sleep 0.3; cap hint_selected_filter_a   # label a: A filter, chip in the status line
keys ']' ']' ']'; sleep 0.4; cap hint_filter_up
keys '[' ; sleep 0.3
keys f w; sleep 0.3                              # crossfader
keys '\;'; sleep 0.3                            # back to A filter (tmux needs ; escaped)
keys ']'; sleep 0.3
keys '\;' ']'; sleep 0.3                        # crossfader again
wheel 64 3; sleep 0.3                            # wheel up edits the selected crossfader
wheel 65 1; sleep 0.3
cap hint_wheel_crossfader
keys '\'; sleep 0.3; cap hint_reset           # crossfader back to 0
keys f c a; sleep 0.3; keys ']' ']'; sleep 0.3  # A loop length 4 -> 8 -> 16
cap hint_loop_len
keys v; sleep 0.5                                # loops 16 beats
keys f x s; sleep 0.3; keys ']' ']'; sleep 0.3   # B pitch +0.2 %
keys f z a; sleep 0.3; keys ']' '['; sleep 0.3   # A gain
keys f Escape; sleep 0.3                         # hints cancelled, selection kept
cap hint_cancelled
keys Escape; sleep 0.3                           # selection cleared
cap hint_cleared
wheel 65 2; sleep 0.4                            # nothing selected: scrolls the library
cap hint_wheel_library
keys Escape; sleep 0.5                           # leaves DJ mode
$T kill-server
kill $FAKE
echo "== hints calls received by the fake"; cat "$W/calls-hints.log"
echo "== codj stderr (hints phase)"; cat "$W/codj-hints.err"
exit 0
