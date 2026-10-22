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
# sequences sent as literal keys), the beat-jump keys w b, Esc.
# Phase 6 drives the learning features: the ? key map, :help <term>, the hint explanations, :tutor
# (the first steps advance as the fake's state changes, Tab skips, Esc pauses, :tutor resumes and
# resets, the progress file lives in the isolated XDG_STATE_HOME) and :mix new through the fake.
# Phase 7 drives the patch decks: F12 opens the patch pane, `:patch new d1` (written and loaded by
# the fake), the sequencer grid (hjkl, x, velocity digits, a capital letter for the pattern, Enter
# queues it, p goes live, u undoes), a hinted patch knob edited with ] [, `:w`, `:patch list`,
# `:patch edit`, `:patch live`, `:patch unload`, `?`, and the checks of what the fake received.
# Phase 8 drives the vim keys: focus (h l), counts (4w 12b 2}), W B, marks (ma 'a `a, :delm),
# loops (v, 8v), 0 gg G, . repeat, + - = pitch with u / Ctrl-r undo, ] [ nudge, :fader :low
# :filter :xfade, the pending count in the status line, the `?` map, Esc never leaving, :q leaving;
# every call is checked in what the fake received.
# `ONLY=6 dbus-run-session -- examples/dj_e2e.sh` runs just phase 6, `ONLY=7` just phase 7, `ONLY=8`
# just phase 8.
# Needs busctl and jq (the fake agent and the conflict step talk to the fake codjd with them).
#
# tmux does not forward the kitty keyboard protocol, so this exercises the no-release fallback.
set -u
[ -n "${DBUS_SESSION_BUS_ADDRESS:-}" ] || { echo "run under dbus-run-session"; exit 2; }
case "$DBUS_SESSION_BUS_ADDRESS" in *"/run/user/"*) echo "that looks like the real session bus"; exit 2;; esac
W=${W:-/tmp/codj-dj-e2e}
BIN=${BIN:-$(dirname "$0")/../target/debug}
export HOME=$W/home XDG_CACHE_HOME=$W/cache XDG_CONFIG_HOME=$W/config XDG_STATE_HOME=$W/state XDG_DATA_HOME=$W/data
mkdir -p "$HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME" "$XDG_STATE_HOME" "$XDG_DATA_HOME"
rm -f "$W"/calls.log "$W"/cap-*
export CODJ_FAKE_ISOLATED=1 CODJ_FAKE_LOG=$W/calls.log
T="tmux -L ${TMUX_SOCK:-codjdje2e}"
cap() { $T capture-pane -p -t $S > "$W/cap-$1.txt"; }
keys() { for k in "$@"; do $T send-keys -t $S -- "$k"; sleep 0.12; done; }
phases_1_to_5() {
"$BIN/examples/fake_codjd" 2>"$W/fake.err" & FAKE=$!
sleep 1
T="tmux -L ${TMUX_SOCK:-codjdje2e}"
$T new-session -d -x 140 -y 40 -s dj "$BIN/codj 2>$W/codj.err; sleep 30"
S=dj
cap() { $T capture-pane -p -t $S > "$W/cap-$1.txt"; }
keys() { for k in "$@"; do $T send-keys -t $S -- "$k"; sleep 0.12; done; }
sleep 2
cap normal
keys M; sleep 0.5; cap entered
keys Enter Down Tab b Tab; sleep 1     # load A (Enter), then B (Tab: library, j/k, b loads)
keys Space l Space h; sleep 1.5; cap playing   # play A, focus B, play B, focus A
keys s m a v; sleep 1; cap sync_mark_loop        # sync A, mark a, loop 4
keys f d ']' ']' Escape; sleep 0.3               # hints: A low up twice
keys ':'; $T send-keys -t $S -l -- 'filter 0.5'; keys Enter; sleep 0.3   # :filter on the focused deck
keys ':'; $T send-keys -t $S -l -- 'xfade -0.5'; keys Enter; sleep 0.3
keys F1 F5 ']'; sleep 0.5; cap mixer             # headphones A, quantize, nudge A
keys c; sleep 0.3; keys c
keys ':'; $T send-keys -t $S -l -- 'delm a'; keys Enter; sleep 0.3
keys '/' p a l; sleep 0.3; cap search            # / types in the library search
keys Enter b; sleep 0.5                          # keep the filter, browse, b loads onto deck B
keys Tab; sleep 0.2
keys ':' q; cap command
keys Enter; sleep 0.5; cap back_to_normal
keys M; sleep 0.3; keys Escape; sleep 0.5; cap esc_stays   # Esc does not leave DJ mode
keys ':' q Enter; sleep 0.5
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
keys Enter Space; sleep 0.5                # Aftertekno onto A, play
keys F6; sleep 1; cap set_armed            # waiting for the mix start
sleep 6; cap set_running                   # started, first items consumed
cmd auto\ on; cap set_auto
keys F8; sleep 0.8; cap set_disarmed
keys F6; sleep 0.8; keys F7; sleep 0.8; cap set_f6_f7
cmd preview\ 2\ 4; cap set_preview
cmd load\ 3; cap set_load
cmd edit; sleep 1.2; cap set_after_edit    # the TUI came back after the editor
cmd q
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
keys Enter Space; sleep 0.8                # Aftertekno onto A, play
cmd cue\ drop; cap cap_cue_a              # A is the only deck playing: SetCue 0
keys Down Tab b Tab; sleep 0.8             # Smokin onto B (not playing)
cmd cue\ hit\ b; cap cap_cue_b           # explicit deck: SetCue 1
cmd cue\ mid\ c; cap cap_cue_bad_deck    # usage note, no call
cmd loopsave\ brk; cap cap_loop_none      # no active loop: daemon's reply as an error
cmd gridsave\ b; cap cap_grid
keys F9; sleep 1.6; cap cap_recording      # strip shows the red REC badge, counting
keys F9; sleep 0.8; cap cap_recorded       # "recorded mix 2: aft → smk, 16 bars, 9 items"
cmd rec\ on; sleep 1; cap cap_rec_on
cmd rec\ off; sleep 0.5
cmd rec; sleep 0.5; cmd rec; sleep 0.5     # toggle on, toggle off
cmd q
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
keys Space; sleep 0.5                  # deck keys are live again
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
keys Enter Down Tab b Tab; sleep 1             # decks loaded
keys w b l w b h; sleep 0.5; cap hint_jump_keys  # DjBeatJump A +1/-1, B +1/-1
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
cmd q                                            # leaves DJ mode
$T kill-server
kill $FAKE
echo "== hints calls received by the fake"; cat "$W/calls-hints.log"
echo "== codj stderr (hints phase)"; cat "$W/codj-hints.err"
}
[ -n "${ONLY:-}" ] || phases_1_to_5


# --- phase 6: the learning features (fresh fake, empty progress)
phase_6() {
export CODJ_FAKE_LOG=$W/calls-learn.log
rm -f "$W"/calls-learn.log
rm -rf "$XDG_STATE_HOME/codj"
rm -f "$HOME/.config/codj/config.toml"
"$BIN/examples/fake_codjd" 2>"$W/fake6.err" & FAKE=$!
sleep 1
S=djlearn
$T new-session -d -x 140 -y 40 -s $S "$BIN/codj 2>$W/codj-learn.err; sleep 30"
sleep 2
cmd() { keys ':'; $T send-keys -t $S -l -- "$1"; sleep 0.15; $T send-keys -t $S Enter; sleep 0.6; }
rep() { n=$1; shift; for i in $(seq "$n"); do keys "$@"; done; }
keys M; sleep 0.5
keys '?'; sleep 0.5; cap help_keys                  # the key map on the screen
keys '?'; sleep 0.3; cap help_closed                # any key closes it, and does nothing else
cmd help\ bass\ swap; cap help_bass_swap            # glossary popup
keys f; sleep 0.3; keys t; sleep 0.3; cap hints_hover   # typed letter: what it leads to
keys a; sleep 0.3; keys f q; sleep 1; cap hint_explained   # selected: its one-liner beside the chip
keys Escape; sleep 0.3                              # clears the selection
cmd help\ flux; cap help_unknown                     # a note, not a popup
# the tutor
cmd tutor; sleep 1.5; cap tutor_1_pick                # a pair proposed from the library, marked
keys BTab a Tab; sleep 0.6; cap tutor_1_loaded_a      # Shift+Tab browses, a loads on A, Tab back; the highlight moved on
keys BTab b BTab; sleep 2; cap tutor_2_play_a         # b loads B: both loaded, step 2 after the dwell
keys Space; sleep 2; cap tutor_3_headphones           # A playing: step 3
keys F2; sleep 2; cap tutor_4_sync                    # B headphones: step 4
keys l s; sleep 2; cap tutor_5_phrase                 # focus B, sync it: step 5 with A's bar counter
keys Escape; sleep 0.5; cap tutor_paused              # pause: the pane is gone, decks untouched
cmd tutor; sleep 0.5; cap tutor_resumed               # resumes at step 5
keys Tab; sleep 0.5; cap tutor_6_skipped              # Tab skips step 5
keys f q; sleep 0.3; rep 20 '['; sleep 2; cap tutor_7_swap  # B low killed: step 7
keys f d; sleep 0.3; rep 20 '['; keys f q; sleep 0.3; keys '\\'; sleep 2; cap tutor_8_fade   # swap done
cmd xfade\ b; keys h Space; sleep 2; cap tutor_9_done  # faded, A focused and stopped: the goodbye
cat "$XDG_STATE_HOME/codj/tutor.json"; echo
keys Escape; sleep 0.3
cmd tutor\ reset; sleep 1; cap tutor_reset           # starts over (both decks are loaded, so step 1 passes by itself)
keys Escape; sleep 0.3
# :mix new through the fake, on the demo set
cmd set\ demo; sleep 0.5
keys l Space; sleep 0.5                                  # B plays, A is stopped: B is on air
cmd mix\ new; cap mix_recipes                         # the recipes with their explanations
keys a; sleep 0.3
cmd mix\ new\ blend\ 8; sleep 1; cap mix_new         # inserted: the note says which mix
cmd errors; cap mix_errors                            # the set's warnings next to its errors
keys Escape; sleep 0.3
cmd q
$T kill-server
kill $FAKE
echo "== learn calls received by the fake"; cat "$W/calls-learn.log"
echo "== codj stderr (learn phase)"; cat "$W/codj-learn.err"
}
case "${ONLY:-}" in ""|6) phase_6;; esac

# --- phase 7: patch decks (fresh fake: four empty slots, patches "acid" and "pad1" on disk)
phase_7() {
export CODJ_FAKE_LOG=$W/calls-patch.log
rm -f "$W"/calls-patch.log
rm -f "$HOME/.config/codj/config.toml"
"$BIN/examples/fake_codjd" 2>"$W/fake7.err" & FAKE=$!
sleep 1
S=djpatch
$T new-session -d -x 140 -y 44 -s $S "$BIN/codj 2>$W/codj-patch.err; sleep 30"
sleep 2
cmd() { keys ':'; $T send-keys -t $S -l -- "$1"; sleep 0.15; $T send-keys -t $S Enter; sleep 0.7; }
keys M; sleep 0.5
keys F12; sleep 0.8; cap patch_pane_empty             # four empty slots, the deck keys are off
cmd patch\ new\ d1; sleep 1; cap patch_new_loaded     # written from the drums template, loaded into P1 (cue-only)
cmd patch\ list; cap patch_list
keys 1; sleep 0.8; cap patch_grid                      # the grid of P1: row names, a moving column
keys l l l j; sleep 0.3                                # cursor to step 4, snare row
keys x; sleep 0.4                                      # toggle the snare on (cell action)
keys 5; sleep 0.3                                      # velocity digit on the same cell
keys l x; sleep 0.3                                    # another cell, with the remembered velocity
keys C; sleep 0.3; keys j j x; sleep 0.3               # edit pattern C: a hat
cap patch_grid_edited
keys Enter; sleep 0.5; cap patch_pattern_queued        # queued: plays at the next bar
keys u; sleep 0.4                                      # undo the hat (PatchUndo)
keys C-r; sleep 0.4                                    # redo it
keys p; sleep 0.8; cap patch_live_badge                # LIVE
keys '?'; sleep 0.4; cap patch_keys_help               # the pane's keys
keys '?'; sleep 0.3
keys Escape; sleep 0.4; cap patch_overview             # back from the grid to the slots
keys f; sleep 0.3; cap patch_hints                     # labels over the knobs and the slot's controls
keys t d; sleep 0.3; keys ']' ']' ']'; sleep 0.5       # P1 knob 1 (kick_level) up three steps
cap patch_knob_edited
keys f t z; sleep 0.3; keys '[' '['; sleep 0.5         # P1's fader down (DjSet deck 2)
cmd w\ d1b; sleep 0.8                                  # PatchSave as d1b, author human
cmd patch\ edit\ d1b; sleep 1; cap patch_editor       # the JSON in the editor pane
keys Escape; sleep 0.5
cmd patch\ live\ 1\ off; sleep 0.6
cmd patch\ unload\ 1; sleep 0.6; cap patch_unloaded
cmd patch\ load\ 3\ pad1; sleep 0.6; cap patch_load_p3
cmd patch\ import\ /nonexistent.ncft\ lead; sleep 0.6; cap patch_import_error
keys Escape; sleep 0.4; cap patch_closed                # the deck keys are back
keys Space; sleep 0.4                                   # deck A play (nothing loaded: still a DjPlay call)
$T kill-server
kill $FAKE
echo "== patch calls received by the fake"; cat "$W/calls-patch.log"
echo "== codj stderr (patch phase)"; cat "$W/codj-patch.err"
P7=0
chk() { if grep -q -- "$2" "$W/calls-patch.log"; then echo "ok   $1"; else echo "FAIL $1 ($2)"; P7=1; fi; }
chk "patch new wrote the template as human, with an empty base hash" 'PatchWrite d1 base= author=human'
chk "and loaded it into the first free slot" '^PatchLoad 0 d1'
chk "a cell was set on the drums" 'PatchAction 0 {"op":"cell","seq":"drums","pattern":0,"step":3,"row":1,"on":true'
chk "a velocity digit sent a cell with a velocity" '"on":true,"velocity":0.56'
chk "pattern C was edited" '"pattern":2,"step":4,"row":3'
chk "Enter queued the pattern for the next bar" '"op":"pattern","seq":"drums","pattern":2,"now":false'
chk "u and Ctrl-r went to the daemon" '^PatchUndo 0'
chk "redo too" '^PatchRedo 0'
chk "p sent the slot live" '^PatchLive 0 true'
chk "a hinted knob was edited as a knob action" '"op":"knob","name":"kick_level"'
chk "the slot fader went out as DjSet on deck 2" '^DjSet fader 2'
chk ":w saved the slot as human" '^PatchSave 0 d1b author=human'
chk ":patch edit read the source" '^PatchSource d1b'
chk ":patch live and unload" '^PatchLive 0 false'
chk "unload" '^PatchUnload 0'
chk "load into P3" '^PatchLoad 2 pad1'
chk "import" '^PatchImport /nonexistent.ncft lead author=human'
[ "$P7" = 0 ] && echo "PHASE 7 OK" || { echo "PHASE 7 FAILED"; return 1; }
}

# --- phase 8: the vim keys (fresh fake)
phase_8() {
export CODJ_FAKE_LOG=$W/calls-vim.log
rm -f "$W"/calls-vim.log
rm -f "$HOME/.config/codj/config.toml"
"$BIN/examples/fake_codjd" 2>"$W/fake8.err" & FAKE=$!
sleep 1
S=djvim
$T new-session -d -x 140 -y 40 -s $S "$BIN/codj 2>$W/codj-vim.err; sleep 30"
sleep 2
cmd() { keys ':'; $T send-keys -t $S -l -- "$1"; sleep 0.15; $T send-keys -t $S Enter; sleep 0.6; }
keys M; sleep 0.5
keys Enter Down Tab b Tab; sleep 1                 # A and B loaded
cap vim_focus_a                                    # A has the focus
keys Space; sleep 0.4                              # DjPlay 0
keys l; sleep 0.3; cap vim_focus_b                 # the badge moves to B
keys Space; sleep 0.4                              # DjPlay 1
keys h; sleep 0.3
keys 4 w; sleep 0.4                                # DjBeatJump 0 4
keys 1 2 b; sleep 0.4                              # DjBeatJump 0 -12
keys 2 '}'; sleep 0.4                              # DjBeatJump 0 128
keys 3 W; sleep 0.4                                # DjBeatJump 0 12
keys '{'; sleep 0.4                                # DjBeatJump 0 -64
keys 4; sleep 0.3; cap vim_count                   # the pending count in the status line
keys Escape; sleep 0.3                             # cancels it
keys m; sleep 0.3; cap vim_pending_mark; keys a; sleep 0.5   # DjHotcue 0 0 set
keys "'" a; sleep 0.5                              # DjHotcue 0 0 jump
keys m c; sleep 0.5; keys '`' c; sleep 0.5         # DjHotcue 0 2 set, then jump
keys 8 v; sleep 0.6; cap vim_loop                  # DjLoop 0 8, the LOOP mode in the status line
keys v; sleep 0.6                                  # DjLoop 0 0
keys 0; sleep 0.3; keys g g; sleep 0.3; keys G; sleep 0.3   # three DjSeek 0
keys s; sleep 0.4; keys .; sleep 0.4               # DjSync 0 twice
keys '+'; sleep 0.3; keys '+'; sleep 1.2           # pitch up twice (one undo step)
keys u; sleep 0.6                                  # DjSet rate 0 back to 1.0000
keys C-r; sleep 0.6                                # and forward again
keys ']'; sleep 0.5; keys '['; sleep 0.8           # nudge A up, then down
keys l; sleep 0.3
keys 3 w; sleep 0.4                                # DjBeatJump 1 3
keys Escape; sleep 0.3; cap vim_esc_stays          # Esc does not leave DJ mode
keys Space; sleep 0.4                              # B was playing: DjPause 1
cmd fader\ 0.5                                     # B (the focus)
cmd low\ kill
cmd filter\ -0.3
cmd xfade\ 1
cmd fader\ a\ 0.2
keys u; sleep 0.6                                  # undo: fader A goes back
cmd delm\ a                                        # DjHotcue 1 0 clear
cmd delm!                                          # all eight on B
cmd focus\ a; cap vim_focus_cmd
keys '?'; sleep 0.4; cap vim_help; keys '?'; sleep 0.3   # the key map, any key closes it
cmd q; sleep 0.4; cap vim_left                     # :q leaves
$T kill-server
kill $FAKE
echo "== vim calls received by the fake"; cat "$W/calls-vim.log"
echo "== codj stderr (vim phase)"; cat "$W/codj-vim.err"
P8=0
L=$W/calls-vim.log
chk() { if grep -q -- "$2" "$L"; then echo "ok   $1"; else echo "FAIL $1 ($2)"; P8=1; fi; }
chkn() { n=$(grep -c -- "$2" "$L"); if [ "$n" -ge "$3" ]; then echo "ok   $1 ($n)"; else echo "FAIL $1 ($2: $n < $3)"; P8=1; fi; }
chkcap() { if grep -q -- "$3" "$W/cap-$2.txt"; then echo "ok   $1"; else echo "FAIL $1 ($2: $3)"; P8=1; fi; }
chkno() { if grep -q -- "$3" "$W/cap-$2.txt"; then echo "FAIL $1 ($2: $3)"; P8=1; else echo "ok   $1"; fi; }
chk "Space plays the focused deck A" '^DjPlay 0$'
chk "l focuses B, Space plays it" '^DjPlay 1$'
chk "4w jumps 4 beats" '^DjBeatJump 0 4$'
chk "12b jumps back 12 beats" '^DjBeatJump 0 -12$'
chk "2} jumps two phrases" '^DjBeatJump 0 128$'
chk "3W jumps three bars" '^DjBeatJump 0 12$'
chk "{ jumps a phrase back" '^DjBeatJump 0 -64$'
chk "ma sets hot cue 0" '^DjHotcue 0 0 set$'
chk "'a jumps to it" '^DjHotcue 0 0 jump$'
chk "mc then \`c" '^DjHotcue 0 2 jump$'
chk "8v loops 8 beats" '^DjLoop 0 8$'
chk "v leaves the loop" '^DjLoop 0 0$'
chkn "0 gg G seek" '^DjSeek 0 ' 3
chkn "s then . sync A twice" '^DjSync 0$' 2
# the fake's decks start at some rate r: ++ gives r+0.2 %, u goes back to r, Ctrl-r forward again
R=($(grep '^DjSet rate 0 ' "$L" | awk '{print $4}'))
if [ "${#R[@]}" -ge 4 ] && awk -v a="${R[0]}" -v b="${R[1]}" -v c="${R[2]}" -v d="${R[3]}" 'BEGIN{exit !(b>a && c<a && d==b)}'; then
  echo "ok   + + raised the pitch, u took the whole step back, Ctrl-r redid it (${R[*]})"
else echo "FAIL pitch + + u Ctrl-r (${R[*]})"; P8=1; fi
chk "] nudges A" '^DjNudge 0 1$'
chk "3w on the focused B" '^DjBeatJump 1 3$'
chk "Esc did not leave: Space paused B" '^DjPause 1$'
chk ":fader on the focused deck" '^DjSet fader 1 0.5000$'
chk ":low kill" '^DjSet low 1 -1.0000$'
chk ":filter" '^DjSet filter 1 -0.3000$'
chk ":xfade" '^DjSet crossfader 0 1.0000$'
chk ":fader a names the deck" '^DjSet fader 0 0.2000$'
chkn "u took the fader back" '^DjSet fader 0 ' 2
chk ":delm a clears the mark on the focus" '^DjHotcue 1 0 clear$'
chkn ":delm! clears all eight" '^DjHotcue 1 . clear$' 8
chkcap "A has the focus at the start" vim_focus_a 'DECK A  ▶ FOCUS'
chkcap "the focus moved to B" vim_focus_b 'DECK B  ▶ FOCUS'
chkno "and left A" vim_focus_b 'DECK A  ▶ FOCUS'
chkcap "the count shows in the status line" vim_count ' 4 *$'
chkcap "a half-typed mark shows" vim_pending_mark ' m *$'
chkcap "the loop mode shows" vim_loop '-- LOOP 8 --'
chkcap "Esc stays in DJ mode" vim_esc_stays 'FOCUS'
chkcap ":focus a" vim_focus_cmd 'DECK A  ▶ FOCUS'
chkcap "the key map is about the focused deck" vim_help 'FOCUSED DECK A keys'
chkno ":q left DJ mode" vim_left 'FOCUS'
[ "$P8" = 0 ] && echo "PHASE 8 OK" || { echo "PHASE 8 FAILED"; return 1; }
}

RC=0
case "${ONLY:-}" in ""|7) phase_7 || RC=1;; esac
case "${ONLY:-}" in ""|8) phase_8 || RC=1;; esac
exit $RC
