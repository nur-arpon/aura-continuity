// What the hero card says about the PC (pure, unit-tested). Built only from events the phone
// already gets: the CDM presence of the PC's Bluetooth link, the Aura Link session and its pipe,
// PC_STATE / lock / unlock results, the Tailscale card.
import 'package:flutter/material.dart';

import 'theme.dart';

enum PcKind { unpaired, unlocking, locked, nearby, away, reachable, outOfReach }

class PcStatus {
  const PcStatus(this.kind, this.title, this.subtitle, this.tone, this.icon);
  final PcKind kind;
  final String title;
  final String subtitle;
  final Tone tone;
  final IconData icon;
}

enum Via { none, bluetooth, tailnet, lan }

Via viaOf(String label) {
  final l = label.toLowerCase();
  if (l.isEmpty) return Via.none;
  if (l.startsWith('rfcomm')) return Via.bluetooth;
  if (l.startsWith('tailnet')) return Via.tailnet;
  return Via.lan;
}

/// "just now", "4 min ago", "2 h ago", "3 days ago".
String ago(int thenMs, int nowMs) {
  if (thenMs <= 0) return 'not yet';
  final s = ((nowMs - thenMs) / 1000).floor();
  if (s < 60) return 'just now';
  if (s < 3600) return '${s ~/ 60} min ago';
  if (s < 86400) return '${s ~/ 3600} h ago';
  final d = s ~/ 86400;
  return d == 1 ? 'yesterday' : '$d days ago';
}

PcStatus pcStatus(Map<String, dynamic> s, Map<String, dynamic> a, {required int nowMs}) {
  if (s['paired'] != true) {
    return const PcStatus(PcKind.unpaired, 'Not paired', 'Pair Aura with your PC to begin', Tone.warn, Icons.link_rounded);
  }
  final via = viaOf((s['linkVia'] as String?) ?? '');
  final near = s['pcNear'] == true || via == Via.bluetooth || via == Via.lan;
  final locked = (s['pcLocked'] as int?) == 1;
  final awayReady = a['code'] == 'ready';
  final how = switch (via) {
    Via.bluetooth => 'Connected over Bluetooth',
    Via.lan => 'Connected over Wi-Fi',
    Via.tailnet => 'Connected via Tailscale',
    Via.none => near ? 'In Bluetooth range' : (awayReady ? 'Reachable via Tailscale' : 'Last seen ${ago((s['lastSeen'] as int?) ?? 0, nowMs)}'),
  };
  if (s['unlockBusy'] == true) {
    return PcStatus(PcKind.unlocking, 'Unlocking…', how, Tone.info, Icons.lock_open_rounded);
  }
  if (locked) {
    return PcStatus(PcKind.locked, 'Locked', '$how · ${ago((s['pcLockedAt'] as int?) ?? 0, nowMs)}', Tone.info, Icons.lock_rounded);
  }
  if (near) return PcStatus(PcKind.nearby, 'Nearby', how, Tone.good, Icons.laptop_chromebook_rounded);
  if (via == Via.tailnet) return PcStatus(PcKind.away, 'Away', '$how · end-to-end encrypted', Tone.info, Icons.public_rounded);
  if (awayReady) return PcStatus(PcKind.reachable, 'Away', how, Tone.info, Icons.public_rounded);
  return PcStatus(PcKind.outOfReach, 'Out of reach', how, Tone.warn, Icons.cloud_off_rounded);
}
