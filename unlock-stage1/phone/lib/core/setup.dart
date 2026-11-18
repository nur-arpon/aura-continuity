// First-run setup state: the "Allow everything" walk (item 25) and the watch step.
// The native side (MainActivity "setupStatus" / "setupRequest" / "watchSetup" / "watchInstall")
// answers a setupRequest only when the user is back in Aura, so the walk simply re-reads the
// checklist after each answer and carries on with the next missing permission.
import 'dart:async';

import 'package:flutter/material.dart';

import 'bridge.dart';
import 'fx.dart';

class SetupMeta {
  const SetupMeta(this.icon, this.title, this.why, {this.runtime = false});
  final IconData icon;
  final String title;
  final String why;

  /// a normal Android dialog (vs a Settings page)
  final bool runtime;
}

const setupMeta = <String, SetupMeta>{
  'notifications': SetupMeta(Icons.notifications_rounded, 'Notifications', 'Shows what arrives from your PC and asks to approve a sign-in.', runtime: true),
  'bluetooth': SetupMeta(Icons.bluetooth_searching_rounded, 'Nearby devices', 'Lets Aura answer your PC over Bluetooth.', runtime: true),
  'location': SetupMeta(Icons.wifi_rounded, 'Wi-Fi name', 'Lets Aura see which Wi-Fi you are on, so Wake PC knows the network of your PC. Location is never recorded.', runtime: true),
  'usage': SetupMeta(Icons.apps_rounded, 'Usage access', 'Tells Aura which app you opened, to carry it on at your PC.'),
  'listener': SetupMeta(Icons.music_note_rounded, 'Media and notification access', 'Reads what is playing so a video resumes at the same second.'),
  'accessibility': SetupMeta(Icons.touch_app_rounded, 'Zero-tap continue', 'Reads a window title in a few apps. Never what you type.'),
  'battery': SetupMeta(Icons.battery_charging_full_rounded, 'Run in the background', 'Keeps Aura ready without polling or draining the battery.'),
  'overlay': SetupMeta(Icons.layers_rounded, 'Display over other apps', 'Lets the small floating Aura chip appear on top of other apps when your PC sends you something to continue.'),
  'bubbles': SetupMeta(Icons.bubble_chart_rounded, 'Bubble notifications', 'Lets Continue from PC also open as an Android bubble. Optional; the floating chip above works without it.'),
};

class SetupItem {
  const SetupItem(this.id, this.granted);
  final String id;
  final bool granted;
}

class SetupController extends ChangeNotifier with WidgetsBindingObserver {
  SetupController(this.bridge) {
    WidgetsBinding.instance.addObserver(this);
  }
  final AuraBridge bridge;

  List<SetupItem> items = const [];
  Set<String> skipped = {};
  String? current;
  bool walking = false;
  bool _disposed = false;

  /// "none" | "app-missing" | "unpaired" | "away" | "connected" | null (not asked yet)
  String? watchState;
  String watchName = '';
  bool watchBusy = false;

  int get grantedCount => items.where((i) => i.granted).length;
  bool get allGranted => items.isNotEmpty && grantedCount == items.length;
  bool granted(String id) => items.any((i) => i.id == id && i.granted);

  /// The next step the walk would ask: first missing one the user has not already declined this run.
  String? nextStep() {
    for (final i in items) {
      if (!i.granted && !skipped.contains(i.id)) return i.id;
    }
    return null;
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed && !walking) unawaited(refresh());
  }

  @override
  void notifyListeners() {
    if (!_disposed) super.notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  Future<void> refresh() async {
    try {
      final r = await bridge.call('setupStatus');
      if (r is List) {
        items = [for (final e in r) SetupItem('${(e as Map)['id']}', e['granted'] == true)];
        notifyListeners();
      }
    } catch (_) {
      // keep the last list; the walk is never a dead end
    }
  }

  /// One tap: every missing permission in turn, back to Aura after each, checklist live.
  Future<void> allowEverything() async {
    if (walking) return;
    walking = true;
    skipped = {};
    notifyListeners();
    try {
      await refresh();
      for (var guard = 0; guard < 20; guard++) {
        final n = nextStep();
        if (n == null) break;
        current = n;
        notifyListeners();
        try {
          await bridge.call('setupRequest', {'id': n});
        } catch (_) {
          skipped.add(n);
        }
        await refresh();
        if (!granted(n)) skipped.add(n);
      }
    } finally {
      walking = false;
      current = null;
      notifyListeners();
    }
    if (allGranted) Fx.success();
  }

  /// Tap on a single row. A runtime permission already declined can only be changed in the app's
  /// Android settings page, so the second tap goes there.
  Future<void> requestOne(String id) async {
    if (walking) return;
    final again = (setupMeta[id]?.runtime ?? false) && skipped.contains(id);
    current = id;
    notifyListeners();
    try {
      await bridge.call('setupRequest', {'id': id, 'appSettings': again});
    } catch (_) {}
    await refresh();
    if (!granted(id)) {
      skipped.add(id);
    } else {
      skipped.remove(id);
    }
    current = null;
    notifyListeners();
  }

  Future<void> refreshWatch() async {
    if (watchBusy) return;
    watchBusy = true;
    notifyListeners();
    try {
      final r = await bridge.call('watchSetup');
      if (r is Map) {
        watchState = '${r['state']}';
        watchName = '${r['name'] ?? ''}';
      }
    } catch (_) {
      watchState ??= 'none';
    }
    watchBusy = false;
    notifyListeners();
  }

  Future<void> installWatchApp() async {
    await bridge.call('watchInstall');
  }
}
