// Pure logic: the hero status, the pairing-code formatter, inbox day groups.
import 'package:flutter_test/flutter_test.dart';
import 'package:unlock_stage1/core/status.dart';
import 'package:unlock_stage1/core/theme.dart';
import 'package:unlock_stage1/screens/inbox.dart';
import 'package:unlock_stage1/screens/onboarding.dart';

import 'support/fake_bridge.dart';

void main() {
  group('pcStatus', () {
    test('unpaired', () {
      expect(pcStatus(unpaired(), {}, nowMs: fixedNow).kind, PcKind.unpaired);
    });
    test('nearby over Bluetooth', () {
      final s = pcStatus(pairedNearby(), awayReady(), nowMs: fixedNow);
      expect(s.kind, PcKind.nearby);
      expect(s.title, 'Nearby');
      expect(s.subtitle, 'Connected over Bluetooth');
      expect(s.tone, Tone.good);
    });
    test('locked away over the tailnet says how and when', () {
      final s = pcStatus(lockedAway(), awayReady(), nowMs: fixedNow);
      expect(s.kind, PcKind.locked);
      expect(s.subtitle, 'Connected via Tailscale · 12 min ago');
    });
    test('away, not connected, Tailscale ready', () {
      final s = pcStatus({...pairedNearby(), 'pcNear': false, 'linkVia': ''}, awayReady(), nowMs: fixedNow);
      expect(s.kind, PcKind.reachable);
      expect(s.subtitle, 'Reachable via Tailscale');
    });
    test('out of reach shows last seen', () {
      final s = pcStatus({...pairedNearby(), 'pcNear': false, 'linkVia': '', 'lastSeen': fixedNow - 3 * 3600000}, awayMissing(), nowMs: fixedNow);
      expect(s.kind, PcKind.outOfReach);
      expect(s.subtitle, 'Last seen 3 h ago');
      expect(s.tone, Tone.warn);
    });
    test('a remote unlock in flight wins', () {
      expect(pcStatus({...lockedAway(), 'unlockBusy': true}, awayReady(), nowMs: fixedNow).kind, PcKind.unlocking);
    });
    test('pipe labels', () {
      expect(viaOf('rfcomm-in'), Via.bluetooth);
      expect(viaOf('tailnet-in 100.1.2.3'), Via.tailnet);
      expect(viaOf('tcp 192.168.1.5'), Via.lan);
      expect(viaOf(''), Via.none);
    });
    test('ago', () {
      expect(ago(0, fixedNow), 'not yet');
      expect(ago(fixedNow - 5000, fixedNow), 'just now');
      expect(ago(fixedNow - 4 * 60000, fixedNow), '4 min ago');
      expect(ago(fixedNow - 25 * 3600000, fixedNow), 'yesterday');
    });
  });

  group('pairing code', () {
    TextEditingValue fmt(String t) => PairCodeFormatter().formatEditUpdate(TextEditingValue.empty, TextEditingValue(text: t));
    test('groups by four, upper-case, drops junk', () {
      expect(fmt('golt7mj2-njfb 3k6x').text, 'GOLT 7MJ2 NJFB 3K6X');
    });
    test('stops at 34 characters', () {
      final t = fmt('A' * 40).text;
      expect(PairCodeFormatter.clean(t).length, 34);
      expect(t.split(' ').last, 'AA');
    });
  });

  test('inbox day groups', () {
    expect(dayOf(fixedNow - 60000, fixedNow), 'Today');
    expect(dayOf(fixedNow - 26 * 3600000, fixedNow), 'Yesterday');
    expect(dayOf(DateTime(2026, 9, 2).millisecondsSinceEpoch, fixedNow), '2 Sep');
  });
}
