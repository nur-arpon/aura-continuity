// Widget tests: every button on every screen reaches the right native call with the right
// feedback, and the flows have no dead ends.
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:unlock_stage1/core/fx.dart';
import 'package:unlock_stage1/core/model.dart';
import 'package:unlock_stage1/core/theme.dart';
import 'package:unlock_stage1/main.dart';
import 'package:unlock_stage1/screens/inbox.dart';
import 'package:unlock_stage1/screens/remote.dart';
import 'package:unlock_stage1/screens/unlock_guide.dart';
import 'package:unlock_stage1/screens/settings.dart';

import 'support/fake_bridge.dart';

Future<void> settle(WidgetTester t, [int frames = 30]) async {
  for (var i = 0; i < frames; i++) {
    await t.pump(const Duration(milliseconds: 50));
  }
}

Future<FakeBridge> boot(WidgetTester t, {FakeBridge? bridge, Widget? home}) async {
  t.view.physicalSize = const Size(412 * 3, 915 * 3);
  t.view.devicePixelRatio = 3;
  addTearDown(t.view.reset);
  final b = bridge ?? FakeBridge();
  final m = AuraModel(b, poll: null, clock: () => fixedNow)..start();
  Fx.played.clear();
  await t.pumpWidget(AuraApp(model: m, home: home));
  await settle(t);
  return b;
}

/// Scrolls the lazy list until [f] is built, then into view.
Future<void> reveal(WidgetTester t, Finder f) async {
  final list = find.byType(Scrollable);
  for (var dir in [-1.0, 1.0]) {
    for (var i = 0; i < 40 && f.evaluate().isEmpty && list.evaluate().isNotEmpty; i++) {
      await t.drag(list.first, Offset(0, 250 * dir));
      await t.pump(const Duration(milliseconds: 50));
    }
  }
  // middle of the screen, never under the toast at the top
  unawaited(Scrollable.ensureVisible(t.element(f), alignment: 0.5));
  await settle(t, 6);
}

Future<void> tapKey(WidgetTester t, String key) async {
  final f = find.byKey(ValueKey(key));
  await reveal(t, f);
  await t.tap(f, warnIfMissed: false);
  await settle(t, 12);
}

void main() {
  group('Open laptop (item 18)', () {
    testWidgets('Home: Open laptop hands the PC tailnet address to the native launcher', (t) async {
      final b = await boot(t);
      await tapKey(t, 'open-laptop');
      expect(b.count('openLaptop'), 1);
      expect(find.text('Opening your laptop in Moonlight'), findsWidgets);
    });

    testWidgets('A locked PC offers Aura unlock first; a second tap opens anyway', (t) async {
      final b = await boot(t);
      b.s = {...b.s, 'pcLocked': 1};
      await AuraScopeTester.model(t).refresh();
      await settle(t, 12);
      await tapKey(t, 'open-laptop');
      expect(b.count('openLaptop'), 0);
      expect(find.text('Unlock PC'), findsWidgets);
      await tapKey(t, 'open-laptop');
      expect(b.count('openLaptop'), 1);
    });

    testWidgets('A successful remote unlock offers Open laptop in its toast', (t) async {
      final b = await boot(t);
      b.a = {...awayReady(), 'resultAt': fixedNow, 'resultOk': true, 'result': 'Your PC is unlocked'};
      await AuraScopeTester.model(t).refresh();
      await settle(t, 12);
      expect(find.text('Show laptop'), findsOneWidget); // the Home tile
      expect(find.text('Open laptop'), findsOneWidget); // the toast action
    });
  });

  group('Remote (item 15)', () {
    testWidgets('Home: Ring my laptop sends the ring, strong feedback, then confirms', (t) async {
      final b = await boot(t);
      await tapKey(t, 'remote-ring');
      expect(b.count('ringPc'), 1);
      expect(b.args['ringPc']?['on'], true);
      expect(Fx.played.first, Cue.send);
      expect(Fx.played, contains(Cue.approved));
      expect(find.text('Ringing your laptop'), findsWidgets);
    });

    testWidgets('Ring failing says why and offers Try again', (t) async {
      final b = FakeBridge()..results['ringPc'] = 'Your PC did not answer. Check Bluetooth, or Tailscale when away';
      await boot(t, bridge: b);
      await tapKey(t, 'remote-ring');
      expect(Fx.played, contains(Cue.error));
      expect(find.text('Try again'), findsOneWidget);
      await t.tap(find.text('Try again'));
      await settle(t, 12);
      expect(b.count('ringPc'), 2);
    });

    testWidgets('Music tab shows what plays and every button sends its command', (t) async {
      final b = await boot(t, home: const RemoteScreen());
      expect(find.text('Kannil Kannil'), findsOneWidget);
      expect(find.text('Kunal Ganjawala'), findsOneWidget);
      expect(find.text('62%'), findsOneWidget);
      expect(find.text('2:33'), findsOneWidget);
      expect(find.text('4:09'), findsOneWidget);
      expect(b.count('remoteWatch'), 1);
      final expected = {'m-prev': 3, 'm-play': 1, 'm-next': 2, 'm-vdown': 5, 'm-mute': 6, 'm-vup': 4};
      for (final e in expected.entries) {
        await tapKey(t, e.key);
        expect(b.args['mediaCmd']?['cmd'], e.value, reason: e.key);
      }
      expect(b.count('mediaCmd'), 6);
      expect(Fx.played.where((c) => c == Cue.tap).length, greaterThanOrEqualTo(6));
    });

    testWidgets('Music tab with nothing playing is calm, not broken', (t) async {
      final b = FakeBridge()..results['remoteState'] = nowPlaying(title: '', artist: '', playing: false);
      await boot(t, bridge: b, home: const RemoteScreen());
      expect(find.text('Nothing playing'), findsOneWidget);
      expect(find.text('Start something on your PC'), findsOneWidget);
    });

    testWidgets('A refused media command shows the PC reason', (t) async {
      final b = FakeBridge()..results['mediaCmd'] = 'Media controls are switched off on the PC (Aura > Remote)';
      await boot(t, bridge: b, home: const RemoteScreen());
      await tapKey(t, 'm-play');
      expect(find.text('Media controls are switched off on the PC (Aura > Remote)'), findsOneWidget);
      expect(Fx.played, contains(Cue.error));
    });

    testWidgets('Slides tab: every button gives the strong slide haptic and sends its key', (t) async {
      final b = await boot(t, home: const RemoteScreen(initialTab: 'slides'));
      final expected = {'p-next': 1, 'p-prev': 2, 'p-black': 3, 'p-start': 5, 'p-end': 6, 'p-white': 4};
      for (final e in expected.entries) {
        Fx.played.clear();
        await tapKey(t, e.key);
        expect(b.args['present']?['cmd'], e.value, reason: e.key);
        expect(Fx.played.first, Cue.slide, reason: e.key);
      }
      expect(b.args['presenterKeys']?['on'], true, reason: 'volume keys become slide keys while the Slides tab is open');
    });

    testWidgets('Volume keys switch can be turned off, switching tabs hands the keys back', (t) async {
      final b = await boot(t, home: const RemoteScreen(initialTab: 'slides'));
      await tapKey(t, 'p-volkeys');
      expect(b.args['presenterKeys']?['on'], false);
      await tapKey(t, 'seg-Music');
      expect(b.args['presenterKeys']?['on'], false);
    });

    testWidgets('Find tab: ring, and the PC-rings-phone switch', (t) async {
      final b = await boot(t, home: const RemoteScreen(initialTab: 'find'));
      await tapKey(t, 'find-laptop');
      expect(b.args['ringPc']?['on'], true);
      await tapKey(t, 'allow-ring');
      expect(b.args['setAllowRing']?['on'], false);
    });

    testWidgets('A ringing phone can be stopped from the app', (t) async {
      final b = FakeBridge()..results['remoteState'] = {...nowPlaying(), 'phoneRinging': true};
      await boot(t, bridge: b, home: const RemoteScreen(initialTab: 'find'));
      await tapKey(t, 'stop-phone-ring');
      expect(b.count('stopRinging'), 1);
    });

    testWidgets('Closing the Remote screen tells the PC to stop sending', (t) async {
      final b = await boot(t, home: const RemoteScreen());
      await t.pumpWidget(const SizedBox());
      expect(b.count('remoteStop'), 1);
    });

    testWidgets('Stay unlocked guide: both buttons open the right Settings page', (t) async {
      final b = await boot(t, home: const UnlockGuideScreen());
      expect(find.text('Add MYLAPTOP'), findsOneWidget);
      await t.tap(find.text('Open Bluetooth'));
      await settle(t, 8);
      expect(b.args['openSettings']?['page'], 'bluetooth');
      await t.tap(find.text('Open security settings').first);
      await settle(t, 8);
      expect(b.args['openSettings']?['page'], 'security');
    });

    testWidgets('Settings: the guide and Remote rows open their screens', (t) async {
      await boot(t, home: const SettingsScreen());
      await tapKey(t, 'stay-unlocked');
      expect(find.text('Stay unlocked'), findsWidgets);
      await t.tap(find.byKey(const ValueKey('back')));
      await settle(t, 14);
      await tapKey(t, 'open-remote');
      expect(find.text('Ring my laptop'), findsWidgets);
    });

    test('clock format', () {
      expect(fmtClock(153390), '2:33');
      expect(fmtClock(3723000), '1:02:03');
      expect(fmtClock(-5), '0:00');
    });
  });

  group('Home', () {
    testWidgets('shows the PC at a glance', (t) async {
      await boot(t);
      expect(find.text('Aura'), findsOneWidget);
      expect(find.text('Nearby'), findsOneWidget);
      expect(find.text('Connected over Bluetooth'), findsOneWidget);
      expect(find.text('MYLAPTOP'), findsOneWidget);
      expect(find.text('Slugterra S2E7 - The Return of the Shane Gang'), findsOneWidget);
    });

    testWidgets('Lock PC: thunk at once, then the result', (t) async {
      final b = await boot(t);
      await tapKey(t, 'lock');
      expect(b.count('awayLock'), 1);
      expect(Fx.played.first, Cue.lock);
      expect(Fx.played, contains(Cue.approved));
      expect(find.text('Locked'), findsWidgets);
    });

    testWidgets('Lock PC failing says so and offers Try again', (t) async {
      final b = FakeBridge()..results['awayLock'] = 'PC not reachable: timed out';
      await boot(t, bridge: b);
      await tapKey(t, 'lock');
      expect(Fx.played, contains(Cue.error));
      expect(find.text('Try again'), findsOneWidget);
      await t.tap(find.text('Try again'));
      await settle(t, 12);
      expect(b.count('awayLock'), 2);
    });

    testWidgets('Unlock PC opens the fingerprint prompt', (t) async {
      final b = await boot(t);
      await tapKey(t, 'unlock');
      expect(b.count('awayUnlock'), 1);
      expect(Fx.played, contains(Cue.tap));
    });

    testWidgets('Continue from laptop, Continue on PC, Clipboard to PC, refresh', (t) async {
      final b = await boot(t);
      await tapKey(t, 'from-laptop');
      expect(b.count('awayContinue'), 1);
      await tapKey(t, 'topc');
      expect(b.count('continueOnPc'), 1);
      await tapKey(t, 'clip');
      expect(b.count('sendClipboard'), 1);
      expect(Fx.played, containsAllInOrder([Cue.send, Cue.approved]));
      await tapKey(t, 'hero');
      expect(b.count('awayRefresh'), 1);
    });

    testWidgets('nothing from the PC yet: the card asks the PC', (t) async {
      final b = FakeBridge(a: awayMissing());
      await boot(t, bridge: b);
      expect(find.text('Nothing to continue yet'), findsOneWidget);
      await tapKey(t, 'from-laptop');
      expect(b.count('awayRefresh'), 1);
      expect(b.count('awayContinue'), 0);
      await tapKey(t, 'away-card');
      expect(b.count('tailscale'), 1);
    });

    testWidgets('approve and deny a PC request', (t) async {
      final b = FakeBridge(s: {...pairedNearby(), 'pending': 'rfcomm', 'pendingLevel': 2});
      await boot(t, bridge: b);
      expect(find.text('Your PC asks to unlock'), findsOneWidget);
      await tapKey(t, 'approve');
      expect(b.count('approve'), 1);
      expect(Fx.played, contains(Cue.approved));
      await tapKey(t, 'deny');
      expect(b.count('deny'), 1);
    });

    testWidgets('update row appears on Home and Settings and installs on tap', (t) async {
      final none = await boot(t);
      expect(find.byKey(const ValueKey('update-home')), findsNothing);
      expect(none.count('installUpdate'), 0);
      final b = FakeBridge(s: {...pairedNearby(), 'updateBuild': 20, 'updateVersion': '1.0.0'});
      b.results['installUpdate'] = 'ok';
      await boot(t, bridge: b);
      expect(find.text('Update to build 20'), findsOneWidget);
      await tapKey(t, 'update-home');
      expect(b.count('installUpdate'), 1);
    });

    testWidgets('finish setup row asks for permissions', (t) async {
      final b = FakeBridge(
        s: {
          ...pairedNearby(),
          'missingPerms': ['android.permission.BLUETOOTH_CONNECT'],
        },
      );
      await boot(t, bridge: b);
      await tapKey(t, 'finish-setup');
      expect(b.count('requestPerms'), 1);
    });

    testWidgets('a remote unlock result shows up in the app', (t) async {
      final b = await boot(t);
      b.a = {...awayReady(), 'result': 'Your PC is unlocked', 'resultOk': true, 'resultAt': fixedNow};
      await AuraScopeTester.model(t).refresh();
      await settle(t, 10);
      expect(find.text('Your PC is unlocked'), findsOneWidget);
    });
  });

  group('Inbox', () {
    testWidgets('opens from Home, marks seen, opens and removes items', (t) async {
      final b = await boot(t);
      await tapKey(t, 'open-inbox');
      await settle(t);
      expect(find.byType(InboxScreen), findsOneWidget);
      expect(b.count('inboxSeen'), 1);
      expect(find.text('Today'), findsOneWidget);
      expect(find.text('Yesterday'), findsOneWidget);
      await t.tap(find.text('Screenshot 2026-10-21 091204.png'));
      await settle(t, 8);
      expect(b.args['inboxOpen']?['id'], 5);
      await t.drag(find.text('docs.flutter.dev'), const Offset(-600, 0));
      await settle(t);
      expect(b.args['inboxRemove']?['id'], 4);
      await tapKey(t, 'clear-inbox');
      await t.tap(find.text('Clear').last);
      await settle(t);
      expect(b.count('inboxClear'), 1);
      expect(find.text('Nothing handed off yet'), findsOneWidget);
      await tapKey(t, 'back');
      expect(find.byType(InboxScreen), findsNothing);
    });

    testWidgets('a clipboard item explains how to paste', (t) async {
      final b = FakeBridge()..results['inboxOpen'] = 'It is on the clipboard: long-press any text field and Paste';
      await boot(t, bridge: b, home: const InboxScreen());
      await t.tap(find.text('Clipboard from your PC'));
      await settle(t, 8);
      expect(find.textContaining('long-press any text field'), findsOneWidget);
    });
  });

  group('Appearance and feel', () {
    testWidgets('Expressive is the default; Classic and a seed colour are saved and applied', (t) async {
      final b = await boot(t, home: const SettingsScreen());
      expect(AuraScopeTester.model(t).expressive, true);
      await tapKey(t, 'seed-${auraSeeds[2].toRadixString(16)}');
      expect(b.args['setUi']?['seed'], auraSeeds[2]);
      await tapKey(t, 'seed-wallpaper');
      expect(b.args['setUi']?['seed'], 0);
      await tapKey(t, 'classic-look');
      expect(b.args['setUi']?['look'], 'classic');
      expect(AuraScopeTester.model(t).expressive, false);
      expect(Theme.of(t.element(find.byType(Scaffold).first)).extension<AuraColors>()!.expressive, false);
    });

    testWidgets('turning the volume dial gives a detent per notch, even inside a scrolling list', (t) async {
      await boot(t, home: const RemoteScreen());
      final dial = find.byKey(const ValueKey('vol-dial'));
      await reveal(t, dial);
      Fx.played.clear();
      final g = await t.startGesture(t.getCenter(dial) + const Offset(60, 0));
      for (var k = 1; k <= 12; k++) {
        await g.moveBy(const Offset(-2, 8));   // mostly vertical: a plain pan would lose to the list
        await t.pump(const Duration(milliseconds: 16));
      }
      await g.up();
      expect(Fx.played.where((c) => c == Cue.detent).length, greaterThan(2));
      await settle(t, 30);
    });
  });

  group('Settings', () {
    testWidgets('every control reaches the phone side', (t) async {
      final b = await boot(t);
      await tapKey(t, 'open-settings');
      await settle(t);
      expect(find.byType(SettingsScreen), findsOneWidget);
      await tapKey(t, 'seg-Fingerprint');
      expect(b.args['setMode']?['mode'], 'fingerprint');
      await tapKey(t, 'seg-Light');
      expect(b.args['setHaptics']?['level'], 1);
      expect(Fx.played, contains(Cue.lock)); // preview of the new strength
      await tapKey(t, 'sound');
      expect(b.args['setSound']?['on'], false);
      await tapKey(t, 'suggest');
      expect(b.args['setSuggest']?['on'], false);
      await tapKey(t, 'clipsync');
      expect(b.args['setClipSync']?['on'], false);
      await tapKey(t, 'testmode');
      expect(b.args['setAuto']?['on'], true);
      await tapKey(t, 'start');
      await tapKey(t, 'stop');
      expect(b.calls, containsAllInOrder(['start', 'stop']));
      await tapKey(t, 'log');
      expect(find.text('Log'), findsWidgets);
    });

    testWidgets('bubble switch and per-app link choice reach the phone side', (t) async {
      final b = await boot(t);
      await tapKey(t, 'open-settings');
      await settle(t);
      await tapKey(t, 'bubble');
      expect(b.args['setBubble']?['on'], false);
      await tapKey(t, 'link-youtube');
      expect(b.args['setLinkMode'], {'app': 'youtube', 'mode': 'browser'});
      await tapKey(t, 'link-linkedin');
      expect(b.args['setLinkMode'], {'app': 'linkedin', 'mode': 'app'});
    });

    testWidgets('chat head switch reaches the phone side', (t) async {
      final b = await boot(t);
      await tapKey(t, 'open-settings');
      await settle(t);
      await tapKey(t, 'chathead');
      expect(b.args['setChatHead']?['on'], false);
    });

    testWidgets('watch pairing request and unpair', (t) async {
      final b = FakeBridge(s: {...pairedNearby(), 'watchPendingFp': '48B1-85F0-F7AC'});
      await boot(t, bridge: b, home: const SettingsScreen());
      await reveal(t, find.text('Same code'));
      await t.tap(find.text('Same code'));
      await settle(t, 8);
      expect(b.count('watchConfirm'), 1);
    });

    testWidgets('setup rows: companion link, battery, media access, Tailscale', (t) async {
      final b = FakeBridge(
        s: {...pairedNearby(), 'companion': false, 'batteryUnrestricted': false},
        h: {'clipSync': false, 'suggest': true, 'mediaAccess': false},
        a: awayMissing(),
      );
      await boot(t, bridge: b, home: const SettingsScreen());
      await tapKey(t, 'companion');
      expect(b.count('linkCompanion'), 1);
      await tapKey(t, 'battery');
      expect(b.count('batterySettings'), 1);
      await tapKey(t, 'media-access');
      expect(b.count('mediaAccess'), 1);
      await tapKey(t, 'away');
      expect(b.count('tailscale'), 1);
    });

    testWidgets('Forget this PC asks first, Keep keeps it, Forget goes back to onboarding', (t) async {
      final b = await boot(t);
      await tapKey(t, 'open-settings');
      await settle(t);
      await tapKey(t, 'forget');
      await tapKey(t, 'forget-cancel');
      expect(b.count('unpair'), 0);
      await tapKey(t, 'forget');
      await tapKey(t, 'forget-confirm');
      await settle(t);
      expect(b.count('unpair'), 1);
      expect(find.text('Get started'), findsOneWidget);
    });
  });

  group('Onboarding', () {
    testWidgets('welcome -> pair -> ready -> done -> home', (t) async {
      final b = await boot(
        t,
        bridge: FakeBridge(s: unpaired(), a: awayMissing(), inbox: []),
      );
      expect(find.text('Continuity for your PC'), findsOneWidget);
      await tapKey(t, 'ob-start');
      await tapKey(t, 'role-main');
      expect(find.text('Which phone is this?'), findsOneWidget);
      // Pair stays disabled until the code is complete
      await t.tap(find.byKey(const ValueKey('ob-pair')));
      await settle(t, 4);
      expect(b.count('pair'), 0);
      await t.enterText(find.byKey(const ValueKey('ob-code')), 'golt7mj2njfb3k6xd5bumsnuqaldlh6lbw');
      await settle(t, 6);
      expect(find.text('34 / 34'), findsOneWidget);
      await tapKey(t, 'ob-pair');
      await settle(t);
      expect(b.args['pair']?['code'], 'GOLT 7MJ2 NJFB 3K6X D5BU MSNU QALD LH6L BW');
      expect(Fx.played, contains(Cue.success));
      expect(find.text('Paired with MYLAPTOP'), findsOneWidget);
      await tapKey(t, 'ob-link');
      expect(b.count('linkCompanion'), 1);
      // one tap walks every permission in order and the checklist ends full
      await tapKey(t, 'ob-perms');
      await settle(t, 20);
      expect(b.setupAsked, b.setupOrder);
      expect(find.text('9 of 9 allowed'), findsOneWidget);
      await tapKey(t, 'ob-continue');
      await settle(t);
      // watch step: connected watch shows its name
      expect(find.text('Galaxy Watch is connected'), findsOneWidget);
      expect(b.count('watchSetup'), greaterThan(0));
      await tapKey(t, 'ob-watch-next');
      await settle(t);
      expect(find.text("You're all set"), findsOneWidget);
      await tapKey(t, 'ob-done');
      await settle(t);
      expect(find.text('Nearby'), findsOneWidget);
    });

    testWidgets('a wrong code shakes, says so, and stays', (t) async {
      final b = await boot(
        t,
        bridge: FakeBridge(s: unpaired(), a: awayMissing(), inbox: []),
      );
      await tapKey(t, 'ob-start');
      await tapKey(t, 'role-main');
      await t.enterText(find.byKey(const ValueKey('ob-code')), 'BAD${'A' * 31}');
      await settle(t, 4);
      await tapKey(t, 'ob-pair');
      await settle(t);
      expect(b.count('pair'), 1);
      expect(Fx.played, contains(Cue.error));
      expect(find.textContaining('That code is not right'), findsOneWidget);
      expect(find.text('Which phone is this?'), findsOneWidget);
      await tapKey(t, 'ob-back');
      await settle(t);
      expect(find.text('Get started'), findsOneWidget);
    });

    testWidgets('the setup step can be skipped (no dead end)', (t) async {
      final b = await boot(
        t,
        bridge: FakeBridge(s: unpaired(), a: awayMissing(), inbox: []),
      );
      await tapKey(t, 'ob-start');
      await tapKey(t, 'role-main');
      await t.enterText(find.byKey(const ValueKey('ob-code')), 'A' * 34);
      await settle(t, 4);
      await tapKey(t, 'ob-pair');
      await settle(t);
      await tapKey(t, 'ob-later');
      await settle(t);
      // no watch: the step says so and can be skipped
      expect(b.count('setupRequest'), 0);
      await tapKey(t, 'ob-watch-next');
      await settle(t);
      expect(find.text("You're all set"), findsOneWidget);
    });

    testWidgets('a declined permission is skipped (no nagging loop), the rest still go through', (t) async {
      final b = await boot(
        t,
        bridge: FakeBridge(s: unpaired(), a: awayMissing(), inbox: []),
      );
      b.setupDeny.add('usage');
      await tapKey(t, 'ob-start');
      await tapKey(t, 'role-main');
      await t.enterText(find.byKey(const ValueKey('ob-code')), 'A' * 34);
      await settle(t, 4);
      await tapKey(t, 'ob-pair');
      await settle(t);
      await tapKey(t, 'ob-perms');
      await settle(t, 20);
      expect(b.setupAsked.where((e) => e == 'usage').length, 1);
      expect(b.setupAsked.length, 9);
      expect(find.text('8 of 9 allowed'), findsOneWidget);
      expect(find.text('Not allowed yet. Tap to try again.'), findsOneWidget);
      // the row itself can be retried
      b.setupDeny.clear();
      await tapKey(t, 'perm-usage');
      await settle(t, 10);
      expect(find.text('9 of 9 allowed'), findsOneWidget);
    });

    testWidgets('watch step: app missing offers the Play listing; no watch can be skipped', (t) async {
      final b = await boot(
        t,
        bridge: FakeBridge(s: unpaired(), a: awayMissing(), inbox: []),
      );
      b.watchState = 'app-missing';
      await tapKey(t, 'ob-start');
      await tapKey(t, 'role-main');
      await t.enterText(find.byKey(const ValueKey('ob-code')), 'A' * 34);
      await settle(t, 4);
      await tapKey(t, 'ob-pair');
      await settle(t);
      await tapKey(t, 'ob-later');
      await settle(t);
      expect(find.text('Install Aura on your watch'), findsOneWidget);
      await tapKey(t, 'ob-watch-install');
      expect(b.count('watchInstall'), 1);
      await tapKey(t, 'ob-watch-next');
      await settle(t);
      expect(find.text("You're all set"), findsOneWidget);
    });

    testWidgets('pair step offers Scan QR and keeps typing as the fallback', (t) async {
      await boot(
        t,
        bridge: FakeBridge(s: unpaired(), a: awayMissing(), inbox: []),
      );
      await tapKey(t, 'ob-start');
      await tapKey(t, 'role-main');
      expect(find.byKey(const ValueKey('ob-scan')), findsOneWidget);
      expect(find.byKey(const ValueKey('ob-code')), findsOneWidget);
    });

    testWidgets('pair stays off until the user says main or extra, and the PC button names are shown', (t) async {
      final b = await boot(t, bridge: FakeBridge(s: unpaired(), a: awayMissing(), inbox: []));
      await tapKey(t, 'ob-start');
      expect(find.textContaining('Replace main phone', findRichText: true), findsOneWidget);
      expect(find.textContaining('Add another phone', findRichText: true), findsOneWidget);
      await t.enterText(find.byKey(const ValueKey('ob-code')), 'A' * 34);
      await settle(t, 6);
      await t.tap(find.byKey(const ValueKey('ob-pair')));
      await settle(t, 4);
      expect(b.count('pair'), 0, reason: 'no phone type chosen yet');
      await tapKey(t, 'role-extra');
      await tapKey(t, 'ob-pair');
      await settle(t);
      expect(b.args['pair']?['role'], 'extra');
    });

    testWidgets('a main-phone pair sends role main', (t) async {
      final b = await boot(t, bridge: FakeBridge(s: unpaired(), a: awayMissing(), inbox: []));
      await tapKey(t, 'ob-start');
      await tapKey(t, 'role-main');
      await t.enterText(find.byKey(const ValueKey('ob-code')), 'A' * 34);
      await settle(t, 6);
      await tapKey(t, 'ob-pair');
      await settle(t);
      expect(b.args['pair']?['role'], 'main');
    });
  });
}

abstract final class AuraScopeTester {
  static AuraModel model(WidgetTester t) => AuraScope.read(t.element(find.byType(Scaffold).first));
}
