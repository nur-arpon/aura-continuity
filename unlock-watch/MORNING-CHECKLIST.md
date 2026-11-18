# Morning test: watch install + Lock PC end to end
APKs: watch `D:\Claude-Projects\Continuity\unlock-watch\wear\build\outputs\apk\release\wear-release.apk`;
phone is already updated (1.0.0+3: receiver, WatchGate, level-0 lock events). Autolock must be the build that has events.rs
(autolock/STATUS.md "copy ... autolock-next" step) and the PC must be paired (pairing secret) for CU1E to verify.

1. Phone: unlock it, open PC Unlock once (Watch tile appears under the companion row). Keep it unlocked for steps 1-6.
2. Watch: Settings > Developer options > Wireless debugging ON. PC: `adb mdns services`, then `adb connect <_adb-tls-connect address>`.
3. `adb -s <watch> install -r ...\wear-release.apk` (same package + cert as the phone, so the Data Layer links them).
4. `adb -s <watch> logcat -s CUWatch` ; open the app on the watch; allow notifications. Expect `app opened: On wrist ...`.
5. Pair: watch **Pair with phone**. Phone log (`adb -s R5CX00TEST1 logcat -s CUWatchLink`): `pair request from watch ... fingerprint XXXX-XXXX-XXXX`.
   Phone Watch tile shows the same code as the watch -> tap **Same code**. Expect `watch pinned:`; watch shows paired.
6. PC watch for the event: `Get-Content` the autolock log, or run the app's log view. Press **Lock PC** on the watch.
   Expect: phone `pc event ...` / app log `watch -> PC event LOCK_NOW #N delivered in .. ms`; PC log
   `link signal from <phone>: phone event LockNow` then the PC locks (hard cooldown/grace/pause may legitimately block it:
   check the pause switch is off and >60 s since last unlock).
7. Phone locked for a long time is fine now: the event is signed with the no-auth level-0 key (phone log `(no-auth key)`, PC log `(no-auth key, lock-only)`).
8. If it says `NOT acknowledged`: PC listener not up (`link signals: listening on RFCOMM 7c1e0030` missing in the autolock log;
   that line only appears when a pairing secret exists) or PC refused (log line `link signal REFUSED ...` gives the reason: BadMac = wrong key level, Replay, Skew).
9. Off-wrist hint (optional): enable **Lock PC when watch comes off** on the watch, take it off: phone `watch push`, PC `OffWrist`.
10. Battery: next day `adb -s <watch> shell dumpsys batterystats --charged com.spacez.unlock_stage1`.
11. Finish: turn Wireless debugging OFF on the watch.
12. Unlock by watch (WatchGate, wired 2026-10-20): lock the phone and wait > 5 min, watch on wrist + unlocked, then on the PC
    `unlock-pc verify --n 5 --order rfcomm`. Expect `level=tap` wins and phone log `answered by watch (on wrist + unlocked, no tap)`.
    Take the watch off (or lock it): expect `watch did not approve: OFF_WRIST` / `WATCH_LOCKED` and the normal fingerprint prompt.
