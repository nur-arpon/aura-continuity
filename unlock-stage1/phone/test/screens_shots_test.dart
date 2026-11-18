// Renders every screen of Aura at the S23 Ultra's size (412 x 915 dp) in light and dark with the
// real fonts. Always runs (so any overflow or exception fails the build); writes the review
// PNGs only when asked:
//   $env:AURA_SHOTS = '1'; flutter test test/screens_shots_test.dart
// Output: unlock-stage1/phone/screenshots/<name>-<light|dark>.png
import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:unlock_stage1/core/model.dart';
import 'package:unlock_stage1/main.dart';
import 'package:unlock_stage1/screens/inbox.dart';
import 'package:unlock_stage1/screens/onboarding.dart';
import 'package:unlock_stage1/screens/remote.dart';
import 'package:unlock_stage1/screens/unlock_guide.dart';
import 'package:unlock_stage1/screens/settings.dart';

import 'support/fake_bridge.dart';

final _write = Platform.environment['AURA_SHOTS'] == '1';
const _out = 'screenshots';
const _ratio = 2.0;

class Shot {
  const Shot(this.name, this.bridge, {this.home, this.after});
  final String name;
  final FakeBridge Function() bridge;
  final Widget Function(AuraModel m)? home;
  final Future<void> Function(WidgetTester t, AuraModel m)? after;
}

Future<void> _frames(WidgetTester t, [int n = 24]) async {
  for (var i = 0; i < n; i++) {
    await t.pump(const Duration(milliseconds: 50));
  }
}

final shots = <Shot>[
  Shot('01-welcome', () => FakeBridge(s: unpaired(), a: awayMissing(), inbox: [])),
  Shot(
    '02-pair',
    () => FakeBridge(s: unpaired(), a: awayMissing(), inbox: []),
    home: (m) => const Onboarding(initialStep: 1, onDone: _noop),
    after: (t, m) async {
      await t.enterText(find.byKey(const ValueKey('ob-code')), 'GOLT7MJ2NJFB3K6XD5BUMS');
      await _frames(t, 10);
      FocusManager.instance.primaryFocus?.unfocus();
      await _frames(t, 6);
    },
  ),
  Shot(
    '03-pair-wrong',
    () => FakeBridge(s: unpaired(), a: awayMissing(), inbox: []),
    home: (m) => const Onboarding(initialStep: 1, onDone: _noop),
    after: (t, m) async {
      await t.enterText(find.byKey(const ValueKey('ob-code')), 'BAD${'Q' * 31}');
      await _frames(t, 6);
      FocusManager.instance.primaryFocus?.unfocus();
      await t.tap(find.byKey(const ValueKey('ob-pair')));
      await _frames(t, 20);
    },
  ),
  Shot(
    '04-ready',
    () => FakeBridge(
      s: {
        ...pairedNearby(),
        'companion': false,
        'missingPerms': ['android.permission.BLUETOOTH_CONNECT'],
      },
    ),
    home: (m) => const Onboarding(initialStep: 2, onDone: _noop),
  ),
  Shot('05-done', () => FakeBridge(), home: (m) => const Onboarding(initialStep: 4, onDone: _noop)),
  Shot('05b-watch', () => FakeBridge(), home: (m) => const Onboarding(initialStep: 3, onDone: _noop)),
  Shot('06-home-nearby', () => FakeBridge()),
  Shot('07-home-locked-away', () => FakeBridge(s: lockedAway(), a: {...awayReady(), 'lastTitle': 'Lo-fi beats to study to', 'lastSub': 'YouTube · 1:02:41'})),
  Shot(
    '08-home-request',
    () => FakeBridge(
      s: {...pairedNearby(), 'pending': 'Bluetooth', 'pendingLevel': 2, 'pcNear': true, 'linkVia': '', 'companion': false, 'inboxUnseen': 0},
      a: awayMissing(),
      inbox: [],
    ),
  ),
  Shot(
    '09-home-scrolled',
    () => FakeBridge(s: {...pairedNearby(), 'pcNear': false, 'linkVia': '', 'lastSeen': fixedNow - 2 * 3600000}, a: awayMissing()),
    after: (t, m) async {
      await t.drag(find.byKey(const ValueKey('home-list')), const Offset(0, -520));
      await _frames(t, 20);
    },
  ),
  Shot(
    '10-home-toast',
    () => FakeBridge()..results['awayLock'] = 'PC not reachable: timed out',
    after: (t, m) async {
      await t.tap(find.byKey(const ValueKey('lock')));
      await _frames(t, 14);
    },
  ),
  Shot('11-inbox', () => FakeBridge(), home: (m) => const InboxScreen()),
  Shot('12-inbox-empty', () => FakeBridge(inbox: []), home: (m) => const InboxScreen()),
  Shot('13-settings', () => FakeBridge(), home: (m) => const SettingsScreen()),
  Shot(
    '14-settings-2',
    () => FakeBridge(h: {'clipSync': false, 'suggest': true, 'mediaAccess': false}, a: awayMissing()),
    home: (m) => const SettingsScreen(),
    after: (t, m) async {
      await t.drag(find.byKey(const ValueKey('settings-list')), const Offset(0, -760));
      await _frames(t, 20);
    },
  ),
  Shot(
    '15-settings-3',
    () => FakeBridge(s: {...pairedNearby(), 'auto': true}),
    home: (m) => const SettingsScreen(),
    after: (t, m) async {
      await t.drag(find.byKey(const ValueKey('settings-list')), const Offset(0, -3000));
      await _frames(t, 20);
    },
  ),
  Shot(
    '16-remote-music',
    () => FakeBridge(),
    home: (m) => const RemoteScreen(),
    after: (t, m) async {
      await m.refreshRemote();
      await _frames(t, 10);
    },
  ),
  Shot(
    '17-remote-slides',
    () => FakeBridge(),
    home: (m) => const RemoteScreen(initialTab: 'slides'),
    after: (t, m) async {
      await m.refreshRemote();
      await _frames(t, 10);
    },
  ),
  Shot('18-remote-find', () => FakeBridge(), home: (m) => const RemoteScreen(initialTab: 'find'), after: (t, m) async {
    await m.refreshRemote();
    await _frames(t, 10);
  }),
  Shot('19-stay-unlocked', () => FakeBridge(), home: (m) => const UnlockGuideScreen()),
  Shot(
    '20-home-remote',
    () => FakeBridge(),
    after: (t, m) async {
      await t.drag(find.byKey(const ValueKey('home-list')), const Offset(0, -380));
      await _frames(t, 20);
    },
  ),
  Shot(
    '21-home-open-laptop',
    () => FakeBridge(),
    after: (t, m) async {
      await t.drag(find.byKey(const ValueKey('home-list')), const Offset(0, -480));
      await _frames(t, 20);
    },
  ),
];

void _noop() {}

void main() {
  for (final mode in [ThemeMode.light, ThemeMode.dark]) {
    for (final s in shots) {
      testWidgets('${s.name} ${mode.name}', (t) async {
        t.view.physicalSize = const Size(412 * _ratio, 915 * _ratio);
        t.view.devicePixelRatio = _ratio;
        // the S23's status bar and gesture bar
        t.view.padding = const FakeViewPadding(top: 36 * _ratio, bottom: 20 * _ratio);
        addTearDown(t.view.reset);
        final key = GlobalKey();
        final m = AuraModel(s.bridge(), poll: null, clock: () => fixedNow)..start();
        await t.pumpWidget(
          RepaintBoundary(
            key: key,
            child: AuraApp(model: m, themeMode: mode, home: s.home?.call(m)),
          ),
        );
        await _frames(t, 30);
        if (s.after != null) await s.after!(t, m);
        // a fixed point in the orb's breathing, so the shots compare well between runs
        await _frames(t, 4);
        if (_write) {
          await t.runAsync(() async {
            final b = key.currentContext!.findRenderObject()! as RenderRepaintBoundary;
            final img = await b.toImage(pixelRatio: _ratio);
            final data = await img.toByteData(format: ui.ImageByteFormat.png);
            Directory(_out).createSync(recursive: true);
            await File('$_out/${s.name}-${mode.name}.png').writeAsBytes(data!.buffer.asUint8List());
          });
        }
        await t.pumpWidget(const SizedBox());
        await _frames(t, 4);
      });
    }
  }
}
