#!/usr/bin/env bash
# Fault-injection matrix for the stage-1 handshake. Run from Git Bash on the PC with the
# phone on USB. ONLY run when the phone owner says the phone is free: it toggles phone
# radios and kills / reboots the app or phone. Every phone setting it changes is recorded
# first and restored on exit (trap), including on Ctrl+C.
#
# It never locks, sleeps or signs out the PC, and never touches the PC Wi-Fi (that would
# cut this session). PC sleep/wake is therefore a manual case (see README).
#
#   tests/fault-matrix.sh all                 # every automatic case
#   tests/fault-matrix.sh baseline screenoff  # chosen cases
#   N=30 tests/fault-matrix.sh baseline       # attempts per case (default 30)
set -u
export MSYS_NO_PATHCONV=1
HERE="$(cd "$(dirname "$0")/.." && pwd)"
PC="$HERE/pc/target/release/unlock-pc.exe"
ADB="${ADB:-D:/Android-SDK/platform-tools/adb.exe}"
SERIAL="${SERIAL:-R5CX00TEST1}"
A="$ADB -s $SERIAL"
# low_power is restored to what it was; if the owner had Power saving ON, it stays ON
N="${N:-30}"
LOG="$(cygpath -w "$HERE")\logs\fault-matrix-$(date +%Y%m%d-%H%M%S).log"
ORDER="${ORDER:-usb,ble,rfcomm}"
mkdir -p "$HERE/logs"

say() { echo "== $*" | tee -a "$LOG"; }
# --escalate tap: an untrusted AUTO answer is re-asked at TAP level, which the phone's
# 30-minute Test mode answers, so unattended runs never pop a fingerprint prompt.
verify() { "$PC" verify --n "$1" --order "${2:-$ORDER}" --per-ms "${PER:-8000}" --ttl-ms 40000 --interval-ms "${INTERVAL:-1000}" --escalate tap ${WARM:+--warm} --log "$LOG" | tail -1; }

# ---- record phone state, restore on exit
W0=$($A shell settings get global wifi_on | tr -d '\r')
B0=$($A shell settings get global bluetooth_on | tr -d '\r')
P0=$($A shell settings get global low_power | tr -d '\r')
D0=$($A shell settings get global mobile_data | tr -d '\r')
U0=$($A shell svc usb getFunctions 2>&1 | tr -d '\r')
say "phone before: wifi_on=$W0 bluetooth_on=$B0 low_power=$P0 mobile_data=$D0 usb=$U0"
restore() {
  say "restoring phone settings"
  timeout 120 $ADB -s $SERIAL wait-for-device
  [ "$B0" = "1" ] && $A shell svc bluetooth enable >/dev/null 2>&1
  [ "$W0" = "1" ] && $A shell svc wifi enable >/dev/null 2>&1
  [ "$D0" = "1" ] && $A shell svc data enable >/dev/null 2>&1
  $A shell settings put global low_power "${P0:-0}" >/dev/null 2>&1
  [ -n "${USB_TOUCHED:-}" ] && $A shell svc usb setFunctions >/dev/null 2>&1   # back to the default
  "$PC" radio bt on >/dev/null 2>&1
  $A shell dumpsys deviceidle unforce >/dev/null 2>&1
  [ -n "${STOP_AT_END:-1}" ] && $A shell am force-stop com.spacez.unlock_stage1 >/dev/null 2>&1 && say "app stopped (open it and tap Stop to also keep it off after a reboot)"
  say "phone after: wifi_on=$($A shell settings get global wifi_on | tr -d '\r') bluetooth_on=$($A shell settings get global bluetooth_on | tr -d '\r') low_power=$($A shell settings get global low_power | tr -d '\r')"
}
trap restore EXIT

app_up() { $A shell am start -n com.spacez.unlock_stage1/.MainActivity >/dev/null; sleep 2; }
# seconds until a handshake on $1 succeeds again (self-heal / recovery time)
recover() {
  local t0=$(date +%s.%N)
  for i in $(seq 1 60); do
    if "$PC" verify --n 1 --order "$1" --per-ms 6000 --ttl-ms 8000 --escalate tap --log "$LOG" >/dev/null 2>&1; then
      say "recovered on $1 after $(python -c "print(round($(date +%s.%N) - $t0, 1))") s"; return 0; fi
    sleep 1
  done
  say "NOT recovered on $1 within 60 tries"
}

case_baseline()  { say "baseline: BLE only";            say "$(verify "$N" ble)"
                   say "baseline: RFCOMM only";         say "$(verify "$N" rfcomm)"; }
case_warm()      { say "BLE with the link kept open";   say "$(WARM=1 verify "$N" ble)"; }
case_screenoff() { $A shell input keyevent KEYCODE_SLEEP; sleep 5
                   say "phone screen off: BLE";         say "$(verify "$N" ble)"
                   say "phone screen off: RFCOMM";      say "$(verify "$N" rfcomm)"; }
case_doze()      { $A shell input keyevent KEYCODE_SLEEP; $A shell dumpsys deviceidle force-idle >/dev/null; sleep 3
                   say "forced deep Doze: $($A shell dumpsys deviceidle get deep | tr -d '
')"; say "$(verify "$N" ble,rfcomm)"
                   $A shell dumpsys deviceidle unforce >/dev/null; }
case_wifioff()   { case "$SERIAL" in *:*) say "SKIPPED wifioff: adb runs over the network ($SERIAL), turning Wi-Fi off would cut it"; return;; esac
                   $A shell svc wifi disable; $A shell svc data disable; sleep 3
                   say "phone Wi-Fi + mobile data off: BLE then RFCOMM fallback"; say "$(verify "$N" ble,rfcomm)"
                   $A shell svc wifi enable; [ "$D0" = "1" ] && $A shell svc data enable; }
case_batterysaver() { $A shell settings put global low_power 1; sleep 3
                   say "battery saver on";              say "$(verify "$N" ble,rfcomm)"
                   $A shell settings put global low_power "${P0:-0}"; }
case_phonebt()   { $A shell svc bluetooth disable; sleep 3
                   say "phone Bluetooth off (expect fast fail, or USB if tethering is on)"; say "$(verify 3 "$ORDER")"
                   $A shell svc bluetooth enable; recover ble; }
case_pcbt()      { "$PC" radio bt off; sleep 2
                   say "PC Bluetooth radio off";        say "$(verify 3 "$ORDER")"
                   "$PC" radio bt on; recover ble; }
case_appkill()   { $A shell am kill com.spacez.unlock_stage1; sleep 2
                   say "am kill (background kill)";     say "$(verify 3 ble,rfcomm)"
                   $A shell am force-stop com.spacez.unlock_stage1; sleep 2
                   say "force-stop (user 'Force stop')"; say "$(verify 3 ble,rfcomm)"
                   app_up; recover ble; }
case_usb()       { USB_TOUCHED=1; $A shell svc usb setFunctions ncm; sleep 6
                   [ -z "$("$PC" status | grep 'usb tether link: Some')" ] && { $A shell svc usb setFunctions rndis; sleep 6; }
                   say "$("$PC" status | grep usb)"
                   say "USB tethering only";            say "$(verify "$N" usb)"
                   $A shell svc bluetooth disable; sleep 3
                   say "USB with phone Bluetooth dead"; say "$(verify "$N" usb,ble,rfcomm)"
                   $A shell svc bluetooth enable
                   say "USB unplug (simulated: tethering off) mid-run, then BLE takes over"
                   $A shell svc usb setFunctions; sleep 4; say "$(verify 5 usb,ble,rfcomm)"; }
case_adb()       { $A shell input keyevent KEYCODE_SLEEP; sleep 3
                   say "adb forward with the phone locked (evaluation only)"
                   say "$("$PC" verify --n "$N" --order usb-adb --adb "$ADB" --serial "$SERIAL" --log "$LOG" | tail -1)"; }
# Needs the owner holding the phone: the default policy (escalate to FINGERPRINT).
case_policy()    { say "AUTO policy, phone unlocked in hand (expect level=auto, RSSI logged)"
                   say "$("$PC" verify --n "$N" --order ble --interval-ms 2000 --log "$LOG" | tail -1)"
                   say "now lock the phone and wait 5 min, then run: tests/fault-matrix.sh policylocked"; }
case_policylocked() { say "AUTO policy, phone locked > 5 min (expect fingerprint prompt, level=fingerprint)"
                   say "$("$PC" verify --n 3 --order ble,rfcomm --interval-ms 3000 --log "$LOG" | tail -1)"; }
case_reboot()    { say "rebooting phone"; $A reboot; $A wait-for-device
                   until [ "$($A shell getprop sys.boot_completed | tr -d '\r')" = "1" ]; do sleep 2; done
                   say "booted; before the phone is unlocked:"; say "$(verify 2 ble,rfcomm)"
                   say "waiting up to 5 min for the owner to unlock the phone once"; recover ble; }

[ $# -eq 0 ] && { echo "cases: baseline warm screenoff doze wifioff batterysaver phonebt pcbt appkill usb adb reboot policy policylocked | all"; exit 1; }
[ "$1" = "all" ] && set -- baseline warm screenoff doze wifioff batterysaver phonebt pcbt appkill usb adb
say "log: $LOG"
say "turn ON 'Test mode (30 min)' in the phone app before running unattended cases"
for c in "$@"; do timeout 120 $ADB -s $SERIAL wait-for-device; "case_$c"; done
