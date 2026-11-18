import 'dart:convert';
// App state: one ChangeNotifier over the native bridge. It refreshes every 700 ms ONLY while the
// app is in front (cheap map reads, no radio work) and stops in the background. Every action
// gives instant feedback (haptic + sound + toast) and then the real result.
import 'dart:async';

import 'package:flutter/material.dart';

import 'bridge.dart';
import 'fx.dart';
import 'pair_qr.dart';
import 'status.dart';
import 'theme.dart';

class AuraToast {
  AuraToast(this.text, {this.tone = Tone.info, this.icon, this.actionLabel, this.action, this.busy = false}) : id = DateTime.now().microsecondsSinceEpoch;
  final String text;
  final Tone tone;
  final IconData? icon;
  final String? actionLabel;
  final VoidCallback? action;
  final bool busy;
  final int id;
}

class AuraModel extends ChangeNotifier with WidgetsBindingObserver {
  AuraModel(this.bridge, {this.poll = const Duration(milliseconds: 700), int Function()? clock})
    : _clock = clock ?? (() => DateTime.now().millisecondsSinceEpoch) {
    Fx.bridge = bridge;
  }

  final AuraBridge bridge;
  final Duration? poll;
  final int Function() _clock;
  int get now => _clock();

  Map<String, dynamic> s = const {};
  Map<String, dynamic> h = const {};
  Map<String, dynamic> a = const {};
  List<Map<String, dynamic>> inbox = const [];
  /// Remote screen state (now playing, ring, presenter keys); filled only while that screen is open.
  Map<String, dynamic> remote = const {};
  bool loaded = false;
  final Set<String> busy = {};
  final toast = ValueNotifier<AuraToast?>(null);
  Timer? _timer;
  int _lastResultAt = -1;

  bool get paired => s['paired'] == true;

  /// An extra phone: share, continue and control only. It never locks or unlocks the PC.
  bool get extraPhone => s['phoneRole'] == 'extra';

  /// Settings > Appearance. Expressive (Material 3, wallpaper colours) is the default; Classic is the old Aura look.
  bool get expressive => s['uiLook'] != 'classic';

  /// Seed colour for the Expressive look; null = the wallpaper (Material You).
  int? get seed {
    final v = s['uiSeed'];
    return v is int && v != 0 ? v & 0xFFFFFFFF : null;
  }

  Future<void> setLook({bool? expressive, int? seed}) async {
    Fx.tick();
    // apply at once (the poll would take up to 700 ms), then persist natively
    s = {...s, 'uiLook': ?(expressive == null ? null : (expressive ? 'expressive' : 'classic')), 'uiSeed': ?seed};
    notifyListeners();
    await bridge.call('setUi', {'look': ?(expressive == null ? null : (expressive ? 'expressive' : 'classic')), 'seed': ?seed});
  }

  /// Home "Send file": the system picker, then the same path as the share sheet (gives its own feedback).
  Future<void> sendFile() => _run('file', () async {
    await bridge.call('pickFile');
  });
  PcStatus get status => pcStatus(s, a, nowMs: now);
  String get pcName {
    final n = (s['pcName'] as String?) ?? '';
    return n.isEmpty ? 'Your PC' : n;
  }

  /// Set by main(): opens the Remote screen on a tab (used by the "Touchpad" home-screen shortcut).
  void Function(String tab)? openRemoteTab;

  /// A home-screen shortcut opened the app: do what it says (wake / lock / laptop / mouse).
  Future<void> checkShortcut() async {
    final id = '${await bridge.call('takeShortcut').catchError((_) => null) ?? ''}';
    switch (id) {
      case 'wake': await wakePc();
      case 'lock': await lockPc();
      case 'laptop': await openLaptop();
      case 'mouse': openRemoteTab?.call('mouse');
    }
  }

  /// Ask Android to pin a shortcut to the home screen (it shows its own confirmation).
  Future<void> pinShortcut(String id) async {
    final ok = await bridge.call('pinShortcut', {'id': id}).catchError((_) => false) == true;
    show(AuraToast(ok ? 'Confirm on your home screen' : 'This launcher cannot pin shortcuts', tone: ok ? Tone.good : Tone.info, icon: Icons.add_to_home_screen_rounded));
  }

  void start() {
    WidgetsBinding.instance.addObserver(this);
    refresh();
    Future.delayed(const Duration(milliseconds: 900), checkShortcut);
    // Aura Anywhere: ask the PC what it was last doing, once per app open (no background polling)
    bridge.call('awayFetch').catchError((_) => null);
    _arm();
  }

  void _arm() {
    _timer?.cancel();
    if (poll != null) _timer = Timer.periodic(poll!, (_) => refresh());
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      refresh();
      checkShortcut();
      // back in the app: what is the PC doing NOW (the native side keeps it to one ask per 20 s)
      bridge.call('awayFetch').catchError((_) => null);
      _arm();
    } else if (state == AppLifecycleState.paused || state == AppLifecycleState.hidden) {
      _timer?.cancel();
    }
  }

  @override
  void dispose() {
    _timer?.cancel();
    WidgetsBinding.instance.removeObserver(this);
    toast.dispose();
    super.dispose();
  }

  Future<void> refresh() async {
    try {
      final ns = await bridge.status();
      final nh = await bridge.handoff();
      final na = await bridge.away();
      final ni = await bridge.inbox();
      s = ns;
      h = nh;
      a = na;
      inbox = ni;
      final first = !loaded;
      loaded = true;
      _remoteUnlockResult(first);
      notifyListeners();
    } catch (_) {
      // the native side is busy (first frame, update); the next tick catches up
    }
  }

  /// A remote unlock finished (the fingerprint prompt was the native side): say so in the app too.
  void _remoteUnlockResult(bool first) {
    final at = (a['resultAt'] as int?) ?? 0;
    if (first || _lastResultAt < 0) {
      _lastResultAt = at;
      return;
    }
    if (at > _lastResultAt) {
      _lastResultAt = at;
      final ok = a['resultOk'] == true;
      show(
        AuraToast(
          (a['result'] as String?)?.isNotEmpty == true ? a['result'] as String : (ok ? 'Your PC is unlocked' : 'Your PC did not unlock'),
          tone: ok ? Tone.good : Tone.warn,
          icon: ok ? Icons.lock_open_rounded : Icons.error_outline_rounded,
          actionLabel: ok ? 'Open laptop' : null,
          action: ok
              ? () {
                  _laptopAskedAt = DateTime.now(); // just unlocked: the status may lag, do not ask again
                  openLaptop();
                }
              : null,
        ),
      );
    }
  }

  void show(AuraToast t) => toast.value = t;

  Future<void> _run(String key, Future<void> Function() f) async {
    if (busy.contains(key)) return;
    busy.add(key);
    notifyListeners();
    try {
      await f();
    } catch (_) {
      Fx.error();
      show(AuraToast('Something went wrong. Try again.', tone: Tone.warn, icon: Icons.error_outline_rounded));
    } finally {
      busy.remove(key);
      notifyListeners();
      unawaited(refresh());
    }
  }

  // ------------------------------------------------------------------ the big actions

  Future<void> lockPc() => _run('lock', () async {
    Fx.play(Cue.lock);
    show(AuraToast('Locking ${pcName == 'Your PC' ? 'your PC' : pcName}…', busy: true, icon: Icons.lock_rounded));
    final r = '${await bridge.call('awayLock') ?? ''}';
    if (r.startsWith('Lock sent')) {
      Fx.play(Cue.approved);
      show(AuraToast('Locked', tone: Tone.good, icon: Icons.lock_rounded));
    } else {
      Fx.error();
      show(AuraToast(r.isEmpty ? 'Your PC did not answer' : r, tone: Tone.warn, icon: Icons.cloud_off_rounded, actionLabel: 'Try again', action: lockPc));
    }
  });

  // ---- remote power: Wake / Sleep / Restart / Shut down

  String get _pcWord => pcName == 'Your PC' ? 'your PC' : pcName;

  Future<void> wakePc() => _run('wake', () async {
    Fx.play(Cue.send);
    show(AuraToast('Waking $_pcWord…', busy: true, icon: Icons.power_settings_new_rounded));
    await bridge.call('locationEnsure'); // first use asks for the Wi-Fi name; Wake still works by subnet if declined
    final r = '${await bridge.call('wakePc') ?? ''}';
    if (r == 'PC is awake') {
      Fx.play(Cue.approved);
      show(AuraToast(r, tone: Tone.good, icon: Icons.check_circle_rounded));
    } else {
      Fx.error();
      show(AuraToast(r.isEmpty ? 'No answer' : r, tone: Tone.warn, icon: Icons.cloud_off_rounded, actionLabel: 'Try again', action: wakePc));
    }
  });

  /// [action] = restart | shutdown | sleep. The confirm dialog (restart, shut down) is the caller's job.
  Future<void> pcPower(String action) => _run('power-$action', () async {
    Fx.play(Cue.send);
    final doing = switch (action) { 'restart' => 'Restarting', 'shutdown' => 'Shutting down', _ => 'Putting to sleep' };
    show(AuraToast('$doing $_pcWord…', busy: true, icon: Icons.power_settings_new_rounded));
    final r = '${await bridge.call('pcPower', {'action': action}) ?? ''}';
    if (r.startsWith('PC not reachable') || r.startsWith('Not paired') || r.isEmpty || r == 'Unknown action') {
      Fx.error();
      show(AuraToast(r.isEmpty ? 'Your PC did not answer' : r, tone: Tone.warn, icon: Icons.cloud_off_rounded, actionLabel: 'Try again', action: () => pcPower(action)));
    } else {
      Fx.play(Cue.approved);
      show(AuraToast(r, tone: Tone.good, icon: Icons.power_settings_new_rounded));
    }
  });

  /// Settings: remember the phone's current Wi-Fi so Wake can broadcast on it.
  Future<void> rememberWifiForWake() async {
    await bridge.call('locationEnsure');
    final r = '${await bridge.call('wakeRemember') ?? ''}';
    final ok = r.startsWith('Remembered');
    ok ? Fx.success() : Fx.error();
    show(AuraToast(r, tone: ok ? Tone.good : Tone.warn, icon: ok ? Icons.wifi_rounded : Icons.wifi_off_rounded));
  }

  /// Fingerprint every time: the system prompt and the result feedback are native (RemoteUnlockActivity).
  Future<void> unlockPc() => _run('unlock', () async {
    await bridge.call('awayUnlock');
  });

  DateTime? _laptopAskedAt;

  /// Open laptop: Moonlight (the client of the PC's Sunshine host) with the PC's address. If the PC is locked the
  /// stream would be a black sign-in screen, so the existing fingerprint unlock is offered first (a second tap within
  /// 20 s opens anyway). The unlock itself is the unchanged native flow ([unlockPc]).
  Future<void> openLaptop() => _run('laptop', () async {
    Fx.tap();
    final locked = (s['pcLocked'] as int?) == 1; // the live Aura Link state, not the last-activity snapshot
    final again = _laptopAskedAt != null && DateTime.now().difference(_laptopAskedAt!) < const Duration(seconds: 20);
    if (locked && !again) {
      _laptopAskedAt = DateTime.now();
      show(AuraToast('Your PC is locked. Unlock it first, or tap Open laptop again to open anyway.', tone: Tone.info, icon: Icons.lock_rounded, actionLabel: 'Unlock PC', action: unlockPc));
      return;
    }
    _laptopAskedAt = null;
    final r = '${await bridge.call('openLaptop') ?? ''}';
    final ok = r.startsWith('Opening');
    show(AuraToast(r.isEmpty ? 'Could not open the remote app' : r, tone: ok ? Tone.good : Tone.info, icon: Icons.desktop_windows_rounded));
  });

  Future<void> continueFromLaptop() => _run('continue', () async {
    show(AuraToast('Asking your PC…', busy: true, icon: Icons.laptop_chromebook_rounded));
    final r = '${await bridge.call('awayContinue') ?? ''}';
    if (r.startsWith('Opening')) {
      show(AuraToast(r, tone: Tone.good, icon: Icons.play_arrow_rounded));
    } else {
      Fx.error();
      show(
        AuraToast(
          r.isEmpty ? 'Your PC did not answer' : r,
          tone: Tone.warn,
          icon: Icons.cloud_off_rounded,
          actionLabel: 'Try again',
          action: continueFromLaptop,
        ),
      );
    }
  });

  /// "More from your PC": open item [index] on this phone.
  /// [mode] is 'app' / 'browser' for a long-press choice, null = the per-app setting.
  Future<void> openRecent(int index, {String? mode}) => _run('recent$index', () async {
    Fx.tap();
    final r = '${await bridge.call('awayOpenRecent', {'index': index, 'mode': ?mode}) ?? ''}';
    final ok = r.startsWith('Opening');
    if (!ok) Fx.error();
    show(AuraToast(r.isEmpty ? 'Could not open that' : r, tone: ok ? Tone.good : Tone.warn, icon: ok ? Icons.play_arrow_rounded : Icons.cloud_off_rounded));
  });

  Future<void> askPc() => _run('ask', () async {
    Fx.tick();
    final r = '${await bridge.call('awayRefresh') ?? ''}';
    if (r == 'ok') {
      Fx.play(Cue.arrive);
    } else {
      Fx.error();
      show(AuraToast(r.isEmpty ? 'Your PC did not answer' : r, tone: Tone.warn, icon: Icons.cloud_off_rounded));
    }
  });

  /// Continue on PC: a short native moment reads what plays / the clipboard; it gives its own feedback.
  Future<void> continueOnPc() => _run('topc', () async {
    await bridge.call('continueOnPc');
  });

  Future<void> clipboardToPc() => _run('clip', () async {
    Fx.play(Cue.send);
    final r = '${await bridge.call('sendClipboard') ?? ''}';
    final ok = r.startsWith('Copied') || r.startsWith('Already');
    if (ok) {
      Fx.play(Cue.approved);
    } else {
      Fx.error();
    }
    show(
      AuraToast(
        r.isEmpty ? 'Your PC did not answer' : r,
        tone: ok ? Tone.good : Tone.warn,
        icon: ok ? Icons.content_paste_go_rounded : Icons.error_outline_rounded,
      ),
    );
  });

  Future<void> approve() async {
    Fx.play(Cue.approved);
    await bridge.call('approve');
    unawaited(refresh());
  }

  Future<void> deny() async {
    Fx.play(Cue.toggleOff);
    await bridge.call('deny');
    unawaited(refresh());
  }

  // ------------------------------------------------------------------ pairing / setup

  /// true = paired. Plays success / error itself.
  Future<bool> pair(String code, {String role = 'main'}) async {
    final q = PairQr.last;
    final same = q != null && q.code.replaceAll(' ', '') == code.toUpperCase().replaceAll(RegExp('[^A-Z0-9]'), '');
    final ok = await bridge.call('pair', {'code': code, 'role': role, if (same && q.addr != null) 'addr': q.addr}) == true;
    if (ok) {
      Fx.success();
    } else {
      Fx.error();
    }
    await refresh();
    return ok;
  }

  /// Pair a further laptop as a secondary companion. Plays success / error itself.
  Future<bool> addLaptop(String code, PairQr? q) async {
    final ok = await bridge.call('laptopAdd', {'code': code, 'name': q?.name, 'addr': q?.addr, 'tailnet': q?.tailnet}) == true;
    if (ok) {
      Fx.success();
    } else {
      Fx.error();
    }
    await refresh();
    return ok;
  }

  Future<void> requestPerms() async {
    await bridge.call('requestPerms');
    unawaited(refresh());
  }

  /// Tap to update: opens the installer (or first the "install unknown apps" page for Aura).
  Future<void> installUpdate() async {
    final r = '${await bridge.call('installUpdate') ?? ''}';
    if (r != 'ok' && r != 'settings' && r.isNotEmpty) {
      show(AuraToast(r, tone: Tone.warn, icon: Icons.info_outline_rounded));
    }
    unawaited(refresh());
  }

  Future<void> linkCompanion() async {
    final r = '${await bridge.call('linkCompanion') ?? ''}';
    if (r == 'linked') {
      Fx.success();
      show(AuraToast('Aura is now always ready', tone: Tone.good, icon: Icons.check_circle_rounded));
    } else if (r.isNotEmpty) {
      show(AuraToast(r, tone: Tone.warn, icon: Icons.info_outline_rounded));
    }
    unawaited(refresh());
  }

  // ------------------------------------------------------------------ settings

  Future<void> set(String method, Map<String, dynamic> args, {Cue? cue}) async {
    if (cue != null) Fx.play(cue);
    await bridge.call(method, args);
    await refresh();
  }

  Future<void> setHaptics(int level) async {
    await bridge.call('setHaptics', {'level': level});
    await refresh();
    // feel the new strength straight away
    if (level > 0) Fx.play(Cue.lock);
  }

  Future<void> setSound(bool on) async {
    await bridge.call('setSound', {'on': on});
    await refresh();
    Fx.toggle(on);
  }

  Future<void> forgetPc() async {
    Fx.play(Cue.toggleOff);
    await bridge.call('unpair');
    await refresh();
  }

  // ------------------------------------------------------------------ remote (item 15)

  Future<void> refreshRemote() async {
    try {
      final r = await bridge.call('remoteState');
      if (r is Map) {
        remote = r.cast<String, dynamic>();
        notifyListeners();
      }
    } catch (_) {}
  }

  /// The Remote screen opened (or is still open): the PC keeps us posted for 2 minutes.
  Future<void> remoteOpen() async {
    await refreshRemote();
    unawaited(Future(() async {
      try {
        await bridge.call('remoteWatch');
      } catch (_) {}
    }));
  }

  void remoteClose() {
    bridge.call('remoteStop').catchError((_) => null);
  }

  Future<void> setPresenterKeys(bool on) async {
    await bridge.call('presenterKeys', {'on': on});
    remote = {...remote, 'presenterKeys': on};
    notifyListeners();
  }

  Future<void> setAllowRing(bool on) async {
    await bridge.call('setAllowRing', {'on': on});
    remote = {...remote, 'allowRing': on};
    notifyListeners();
  }

  Future<void> stopRinging() async {
    Fx.play(Cue.toggleOff);
    await bridge.call('stopRinging');
    await refreshRemote();
  }

  Future<void> ringLaptop(bool on) => _run('ring', () async {
    Fx.play(Cue.send);
    show(AuraToast(on ? 'Ringing your laptop…' : 'Stopping…', busy: true, icon: Icons.notifications_active_rounded));
    final r = '${await bridge.call('ringPc', {'on': on}) ?? ''}';
    final ok = r == 'Ringing your laptop' || r == 'Stopped';
    if (ok) {
      Fx.play(Cue.approved);
      show(AuraToast(r, tone: Tone.good, icon: on ? Icons.notifications_active_rounded : Icons.notifications_off_rounded));
    } else {
      Fx.error();
      show(AuraToast(r.isEmpty ? 'Your PC did not answer' : r, tone: Tone.warn, icon: Icons.cloud_off_rounded, actionLabel: 'Try again', action: () => ringLaptop(on)));
    }
    await refreshRemote();
  });

  /// Music / volume. Instant tap feedback; a failure says why and offers nothing silent.
  Future<void> mediaCmd(int cmd) async {
    Fx.tap();
    final r = '${await bridge.call('mediaCmd', {'cmd': cmd}) ?? ''}';
    if (r != 'ok') {
      Fx.error();
      show(AuraToast(r, tone: Tone.warn, icon: Icons.cloud_off_rounded, actionLabel: 'Try again', action: () => mediaCmd(cmd)));
    } else {
      await Future<void>.delayed(const Duration(milliseconds: 350));
      await refreshRemote();
    }
  }

  /// One notch of the volume dial: the dial already played its own detent, so no tap cue here, and
  /// the PC's real level is read once the turning stops (not after every notch).
  int _volTurn = 0;
  Future<void> volStep(int cmd) async {
    final turn = ++_volTurn;
    final r = '${await bridge.call('mediaCmd', {'cmd': cmd}) ?? ''}';
    if (r != 'ok') {
      if (turn == _volTurn) { Fx.error(); show(AuraToast(r, tone: Tone.warn, icon: Icons.cloud_off_rounded)); }
      return;
    }
    await Future<void>.delayed(const Duration(milliseconds: 500));
    if (turn == _volTurn) await refreshRemote();
  }

  /// Remote mouse / keyboard: one small event to the PC (no reply, dropped after 2 s).
  void input(Map<String, Object> event) {
    bridge.call('input', {'json': jsonEncode(event)});
  }

  /// Presentation remote: strong haptic first (the room is quiet, the hand must feel it), then the key.
  Future<void> present(int cmd) async {
    Fx.play(Cue.slide);
    final r = '${await bridge.call('present', {'cmd': cmd}) ?? ''}';
    if (r != 'ok') {
      Fx.error();
      show(AuraToast(r, tone: Tone.warn, icon: Icons.error_outline_rounded));
    }
  }

  Future<void> openSettingsPage(String page) async {
    final r = '${await bridge.call('openSettings', {'page': page}) ?? ''}';
    if (r != 'ok' && r.isNotEmpty) show(AuraToast(r, tone: Tone.info, icon: Icons.info_outline_rounded));
  }

  // ------------------------------------------------------------------ inbox

  Future<void> openItem(Map<String, dynamic> it) async {
    Fx.tap();
    final r = '${await bridge.call('inboxOpen', {'id': it['id']}) ?? ''}';
    if (r != 'ok') {
      show(
        AuraToast(r, tone: it['kind'] == 'clip' ? Tone.info : Tone.warn, icon: it['kind'] == 'clip' ? Icons.content_paste_rounded : Icons.info_outline_rounded),
      );
    }
  }

  Future<void> removeItem(Map<String, dynamic> it) async {
    Fx.tick();
    inbox = [
      for (final e in inbox)
        if (e['id'] != it['id']) e,
    ];
    notifyListeners();
    await bridge.call('inboxRemove', {'id': it['id']});
  }

  Future<void> clearInbox() async {
    Fx.play(Cue.toggleOff);
    inbox = const [];
    notifyListeners();
    await bridge.call('inboxClear');
  }

  Future<void> inboxSeen() async {
    if (((s['inboxUnseen'] as int?) ?? 0) == 0) return;
    await bridge.call('inboxSeen');
    unawaited(refresh());
  }
}

/// Puts the model in reach of every screen.
class AuraScope extends InheritedNotifier<AuraModel> {
  const AuraScope({super.key, required AuraModel model, required super.child}) : super(notifier: model);

  static AuraModel of(BuildContext context) => context.dependOnInheritedWidgetOfExactType<AuraScope>()!.notifier!;
  static AuraModel read(BuildContext context) => context.getInheritedWidgetOfExactType<AuraScope>()!.notifier!;
}
