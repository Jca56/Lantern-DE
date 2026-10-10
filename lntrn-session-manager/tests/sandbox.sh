#!/bin/bash
# End-to-end run of lntrn-session-manager against stand-ins, without logging
# out and without touching the live session.
#
#   cargo build -p lntrn-session-manager && lntrn-session-manager/tests/sandbox.sh
#
# The real binary is started over and over inside a sandbox: its own PID
# namespace (the live session's processes cannot be seen, let alone
# signalled), a private tmpfs on /tmp, and $HOME / $XDG_RUNTIME_DIR under it.
# The compositor, the shell daemons, systemctl and
# dbus-update-activation-environment are small scripts that play their part
# and, on request, misbehave. Needs unshare and python3. Takes about 40 s.
set -u

if [ "$$" != 1 ]; then
  BIN=$(realpath "${1:-$(dirname "$0")/../../target/debug/lntrn-session-manager}") || exit 99
  [ -x "$BIN" ] || { echo "no binary at $BIN (build it first)"; exit 99; }
  exec unshare --user --map-root-user --pid --fork --mount --mount-proc bash "$0" "$BIN"
fi
BIN=$1
mount -t tmpfs tmpfs /tmp || exit 99

H=/tmp/home; B=$H/.lantern/bin; L=$H/.lantern/log
PASS=0; FAIL=0
check() { if eval "$2" >/dev/null 2>&1; then PASS=$((PASS+1)); echo "  PASS  $1"; else FAIL=$((FAIL+1)); echo "  FAIL  $1"; fi; }
alive() { [ -d "/proc/$1" ] && ! grep -q '^State:[[:space:]]*Z' "/proc/$1/status" 2>/dev/null; }

# A fresh home with the stand-ins in ~/.lantern/bin and three autostart
# entries: one to run, one the user switched off, one GNOME switched off.
setup() {
  pkill -9 -f 'sleep 600' 2>/dev/null; pkill -9 -f lntrn-compositor 2>/dev/null; pkill -9 -f foreign-listener 2>/dev/null
  rm -rf /tmp/home /tmp/run /tmp/etc /tmp/.X* /tmp/calls.log /tmp/autostart-* /tmp/notifyd-crashed /tmp/out /tmp/stdout.txt
  mkdir -p "$B" "$L" /tmp/run $H/.config/autostart /tmp/etc/xdg/autostart /tmp/.X11-unix
  cat > $B/lntrn-compositor <<'PY'
#!/usr/bin/python3
import os, signal, socket, sys, time
mode = os.environ.get("FAKE_COMPOSITOR", "ok")
print("fake compositor args:", sys.argv[1:], "mode:", mode, flush=True)
if mode == "crash":
    time.sleep(0.3)
    os.kill(os.getpid(), signal.SIGSEGV)
s = socket.socket(socket.AF_UNIX); s.bind(os.environ["XDG_RUNTIME_DIR"] + "/wayland-1"); s.listen(8)
if mode != "noxwayland":
    open("/tmp/.X0-lock", "w").write("%10d\n" % os.getpid())
    x = socket.socket(socket.AF_UNIX); x.bind("/tmp/.X11-unix/X0"); x.listen(8)
def term(*_):
    print("fake compositor: got SIGTERM, exiting cleanly", flush=True); sys.exit(0)
signal.signal(signal.SIGTERM, term)
signal.signal(signal.SIGHUP, signal.SIG_IGN)
while True: time.sleep(1)
PY
  cat > $B/lntrn-desktop <<'SH'
#!/bin/sh
echo "fake lntrn-desktop pid $$ args=[$*] WAYLAND_DISPLAY=$WAYLAND_DISPLAY DISPLAY=${DISPLAY-unset}"
# An lntrn-desktop from before --foreground: forks, and the parent exits 0.
if [ "$FAKE_DESKTOP" = daemonize ]; then sleep 600 & exit 0; fi
exec sleep 600
SH
  cat > $B/lntrn-portal <<'SH'
#!/bin/sh
echo "fake lntrn-portal pid $$"
if [ "$FAKE_PORTAL" = stubborn ]; then trap '' TERM; while :; do sleep 0.05; done; fi
exec sleep 600
SH
  cat > $B/lntrn-notifyd <<'SH'
#!/bin/sh
echo "fake notifyd pid $$ starting"
if [ "$FAKE_NOTIFYD" = always ] || [ ! -e /tmp/notifyd-crashed ]; then touch /tmp/notifyd-crashed; echo "about to segfault" >&2; kill -SEGV $$; fi
exec sleep 600
SH
  for s in systemctl dbus-update-activation-environment; do
    printf '#!/bin/sh\necho "%s $*" >> /tmp/calls.log\n' "$s" > $B/$s
  done
  chmod +x $B/*
  app() { printf '[Desktop Entry]\nType=Application\nExec=%s\n' "$1"; }
  app 'touch /tmp/autostart-hello %U' > $H/.config/autostart/hello.desktop
  app 'touch /tmp/autostart-blocked'  > /tmp/etc/xdg/autostart/blocked.desktop
  { app 'touch /tmp/autostart-blocked'; echo 'Hidden=true'; } > $H/.config/autostart/blocked.desktop
  { app 'touch /tmp/autostart-gnome-off'; echo 'X-GNOME-Autostart-enabled=false'; } > /tmp/etc/xdg/autostart/gnomeoff.desktop
}

# start [FAKE_COMPOSITOR mode] [VAR=value]: stdout goes through a FIFO whose
# reader can be taken away later, the way a hung-up TTY goes away.
start() {
  mkfifo /tmp/out; cat /tmp/out > /tmp/stdout.txt & READER=$!
  ( cd $H && exec env -i HOME=$H XDG_RUNTIME_DIR=/tmp/run PATH=$B:/usr/bin \
      DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/run/no-bus XDG_CONFIG_HOME=$H/.config XDG_CONFIG_DIRS=/tmp/etc/xdg \
      FAKE_COMPOSITOR="${1:-ok}" ${2:+"$2"} "$BIN" > /tmp/out 2>&1 ) & SM=$!
  T0=$(date +%s%N)
}
# await <text in session.log> [tenths of a second]
await() { for _ in $(seq 1 "${2:-150}"); do grep -q "$1" $L/session.log 2>/dev/null && return 0; alive $SM || return 1; sleep 0.1; done; return 1; }
elapsed() { echo $(( ($(date +%s%N) - T0) / 1000000 )); }   # milliseconds
finish() { kill -TERM $SM 2>/dev/null; wait $SM 2>/dev/null; kill $READER 2>/dev/null; }

echo "=== A. full session, a daemon crash, a hung-up terminal at shutdown ==="
setup
( exec -a /usr/lib/xdg-desktop-portal sleep 600 ) & PORTAL=$!
( exec -a /usr/lib/xdg-desktop-portal-gtk sleep 600 ) & PORTAL_GTK=$!
sleep 0.2
start
check "session comes up"                                   'await "Lantern Session running"'
echo "        (up in $(elapsed) ms)"
COMP=$(pgrep -f "python3 $B/lntrn-compositor")
check "wayland socket identified"                          'grep -q "Found Wayland socket: wayland-1" $L/session.log'
check "X11 display identified"                             'grep -q "Found X11 display: :0" $L/session.log'
check "lines are timestamped"                              'grep -qE "^20[0-9]{2}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z .*Lantern Session starting" $L/session.log'
check "previous xdg-desktop-portal stopped"                '! alive $PORTAL'
check "xdg-desktop-portal-gtk left alone"                  'alive $PORTAL_GTK'
check "env exported incl. WAYLAND_DISPLAY and DISPLAY"     'grep -q "dbus-update-activation-environment WAYLAND_DISPLAY=wayland-1" /tmp/calls.log && grep -q "DISPLAY=:0" /tmp/calls.log && grep -q "systemctl --user import-environment XDG_CURRENT_DESKTOP DESKTOP_SESSION XDG_SESSION_TYPE PATH" /tmp/calls.log'
check "desktop started with --foreground + right env"      'grep -q "args=\[--foreground\] WAYLAND_DISPLAY=wayland-1 DISPLAY=:0" $L/lntrn-desktop.log'
check "portal output lands in its own log"                 'grep -q "fake lntrn-portal" $L/lntrn-portal.log'
check "compositor output lands in compositor.log"          'grep -q "fake compositor args: \[.--udev.\]" $L/compositor.log'
check "notifyd crash is logged with its signal"            'await "lntrn-notifyd (pid [0-9]*) ended: killed by SIGSEGV" 30'
check "notifyd is restarted after its delay"               'await "lntrn-notifyd died (killed by SIGSEGV, core dumped), starting it again" 40'
sleep 0.5
NPID=$(sed -n 's/.*Started lntrn-notifyd (pid \([0-9]*\)).*/\1/p' $L/session.log | tail -1)
check "restarted notifyd (pid $NPID) is the one running"   'alive $NPID && [ "$(tr "\0" " " < /proc/$NPID/cmdline)" = "sleep 600 " ]'
check "its log has both runs, crash output kept"           '[ "$(grep -c "fake notifyd pid" $L/lntrn-notifyd.log)" = 2 ] && [ "$(grep -c "about to segfault" $L/lntrn-notifyd.log)" = 1 ]'
check "exactly the three daemons are running"              '[ "$(pgrep -f "sleep 600" | wc -l)" = 3 ]'
check "autostart ran, field code stripped"                 '[ -e /tmp/autostart-hello ] && [ ! -e "$H/%U" ]'
check "user Hidden=true overrides system entry"            '[ ! -e /tmp/autostart-blocked ]'
check "X-GNOME-Autostart-enabled=false honoured"           '[ ! -e /tmp/autostart-gnome-off ]'
kill $READER; wait $READER 2>/dev/null; kill -HUP $SM; wait $SM; RC=$?
check "exits 0 after SIGHUP with a dead terminal"          '[ "$RC" = 0 ]'
check "no panic"                                           '! grep -q PANIC $L/session.log'
check "cleanup ran to the end"                             'grep -q "Cleaning up" $L/session.log && grep -q "Lantern Session ended" $L/session.log'
check "compositor was asked to quit and did"               'grep -q "got SIGTERM, exiting cleanly" $L/compositor.log && ! alive $COMP'
check "no daemon left behind"                              '! pgrep -f "sleep 600" | grep -q .'
echo "--- session.log ---"; sed 's/^/    /' $L/session.log

echo; echo "=== B. compositor dies while starting ==="
setup; start crash
wait $SM; RC=$?
echo "        (gave up after $(elapsed) ms, exit code $RC)"
check "fails fast with the reason"                         'grep -q "FATAL: lntrn-compositor ended while starting (killed by SIGSEGV" $L/session.log && [ "$RC" = 1 ]'
check "nothing else was started"                           '! grep -q "Starting lntrn-desktop" $L/session.log'
check "took under 2s"                                      '[ "$(elapsed)" -lt 2000 ]'
kill $READER 2>/dev/null

echo; echo "=== C. stale and foreign sockets present ==="
setup
python3 -c 'import socket; s=socket.socket(socket.AF_UNIX); s.bind("/tmp/run/wayland-0")'     # stale: bound, nobody listening
python3 -c 'import socket,time; s=socket.socket(socket.AF_UNIX); s.bind("/tmp/run/wayland-7"); s.listen(4); time.sleep(600)  # foreign-listener' &
printf '%10d\n' 1 > /tmp/.X5-lock; python3 -c 'import socket; s=socket.socket(socket.AF_UNIX); s.bind("/tmp/.X11-unix/X5")'
sleep 0.3
start; await "Lantern Session running"
check "picks the compositor's own sockets"                 'grep -q "Found Wayland socket: wayland-1" $L/session.log && grep -q "Found X11 display: :0" $L/session.log'
finish

echo; echo "=== D. XWayland switched off ==="
setup; touch $L/no-xwayland
start noxwayland; await "Lantern Session running"; UP=$(elapsed)
echo "        (up in $UP ms)"
check "skips the X11 wait"                                 'grep -q "XWayland is switched off" $L/session.log && [ "$UP" -lt 1500 ]'
finish

echo; echo "=== E. normal logout (compositor exits by itself), one daemon ignores SIGTERM ==="
setup; start ok FAKE_PORTAL=stubborn
await "Lantern Session running"; sleep 0.3
COMP=$(pgrep -f "python3 $B/lntrn-compositor"); T0=$(date +%s%N)
kill -TERM $COMP; wait $SM; RC=$?
echo "        (session ended $(elapsed) ms after the compositor went, exit code $RC)"
check "compositor exit is recorded with its status"        'grep -q "Compositor ended (exit code 0), shutting down session" $L/session.log'
check "stubborn daemon is killed after the grace period"   'grep -q "lntrn-portal (pid [0-9]*) was still up 2s after SIGTERM, killing it" $L/session.log'
check "session ends cleanly, nothing left"                 'grep -q "Lantern Session ended" $L/session.log && [ "$RC" = 0 ] && ! pgrep -f "sleep 600" | grep -q . && ! pgrep -f "trap" | grep -q .'
kill $READER 2>/dev/null

echo; echo "=== F. an lntrn-desktop that still daemonizes ==="
setup; start ok FAKE_DESKTOP=daemonize
await "Lantern Session running"; sleep 2.5
check "its clean exit is logged"                           'grep -q "lntrn-desktop (pid [0-9]*) ended: exit code 0" $L/session.log'
check "it is not started a second time"                    '! grep -q "lntrn-desktop died" $L/session.log && [ "$(grep -c "Started lntrn-desktop" $L/session.log)" = 1 ]'
finish

echo; echo "=== G. a daemon that cannot stay up ==="
setup; start ok FAKE_NOTIFYD=always
check "gives up after 5 deaths"                            'await "lntrn-notifyd has died 5 times within a minute and stays down" 220'
echo "        (gave up $(elapsed) ms in)"
sleep 3
check "4 restarts (1s, 2s, 4s, 8s), then no more"          '[ "$(grep -c "lntrn-notifyd died" $L/session.log)" = 4 ] && [ "$(grep -c "Started lntrn-notifyd" $L/session.log)" = 5 ]'
check "the rest of the session is unaffected"              'alive $SM && [ "$(pgrep -f "sleep 600" | wc -l)" = 2 ]'
finish

echo; echo "=== RESULT: $PASS passed, $FAIL failed ==="
exit $FAIL
