#!/usr/bin/env bash
# rgp_trigger_watch.sh: take one-frame RGP captures from a running Vulkan game at chosen moments (RADV / Mesa, Linux).
#
# The game must be started with RADV's capture enabled and a trigger file, for example:
#   MESA_VK_TRACE=rgp MESA_VK_TRACE_TRIGGER=/tmp/rgp_trigger RADV_THREAD_TRACE_BUFFER_SIZE=$((256*1024*1024)) \
#   RADV_THREAD_TRACE_QUEUE_EVENTS=1 RADV_THREAD_TRACE_INSTRUCTION_TIMING=0 %command%
# (Check the variable names against your Mesa version; older releases used RADV_THREAD_TRACE* for the mode and trigger too.)
#
# RADV writes one capture (.rgp, usually in /tmp) each time the trigger file appears. This watcher touches the trigger when a line
# matching PATTERN appears in the game's log (e.g. your own "lap start" or "scene begin" marker), then again after each DELAY.
# Triggering from a logged in-game event, not from a planned frame number, is what keeps captures on the frames you meant.
#
# usage: rgp_trigger_watch.sh <log file> <pattern> <out dir> [delays in s, default "6 14"] [max triggers, default 6]
# Run it detached (systemd-run --user or setsid); a plain nohup over ssh can die with the session.
set -u
LOG="$1"; PAT="$2"; OUT="$3"; DELAYS="${4:-6 14}"; MAX="${5:-6}"
TRIG="${MESA_VK_TRACE_TRIGGER:-/tmp/rgp_trigger}"; MARK="$(mktemp /tmp/rgp_mark.XXXXXX)"
mkdir -p "$OUT"; n=0

move_new() {  # move captures newer than the start mark into OUT
  find /tmp -maxdepth 1 -name '*.rgp' -newer "$MARK" -print0 2>/dev/null | while IFS= read -r -d '' f; do
    mv -n "$f" "$OUT/" && echo "$(date +%T) captured $(basename "$f")"
  done
}

echo "$(date +%T) watching $LOG for /$PAT/, delays: $DELAYS, max $MAX"
tail -n0 -F "$LOG" 2>/dev/null | while IFS= read -r line; do
  [[ "$line" =~ $PAT ]] || continue
  for d in $DELAYS; do
    sleep "$d"; touch "$TRIG"; n=$((n+1)); echo "$(date +%T) trigger $n (+${d}s after: ${line:0:80})"
    sleep 3; rm -f "$TRIG"; move_new
    [ "$n" -ge "$MAX" ] && { sleep 5; move_new; rm -f "$MARK"; pkill -P $$ tail 2>/dev/null; exit 0; }
  done
done
