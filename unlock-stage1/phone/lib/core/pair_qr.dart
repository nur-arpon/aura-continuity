// The PC's "Add a phone" QR: `aura://pair?v=1&c=<the same code you could type>&x=<expiry, unix seconds>`.
// Pure parsing, so it is unit-tested without a camera. The QR carries nothing the typed code does not;
// the expiry only makes a photographed screen stop working after a few minutes.
import 'package:flutter/foundation.dart';

enum PairQrIssue { notAura, expired }

@immutable
class PairQr {
  const PairQr(this.code, this.expiresAt, {this.role, this.name, this.addr, this.tailnet});

  /// "companion" for a QR made by "Add another phone" / a laptop's companion pairing; null for the normal one.
  final String? role;

  /// The PC's name, Bluetooth address and tailnet address when the QR carries them (companion QRs do).
  final String? name;
  final String? addr;
  final String? tailnet;

  /// The QR the scanner read last (so the pairing call can use its extra fields).
  static PairQr? last;

  /// The 34 letters/digits, grouped by 4 exactly like a typed code ("GOLT 7MJ2 ...").
  final String code;

  /// Unix seconds, or 0 when the QR had none.
  final int expiresAt;

  static const codeLength = 34;

  /// The phone's clock may differ from the PC's (an old spare phone without network time drifts by minutes).
  static const skewSecs = 900;

  /// A phone clock this far past the QR's expiry is wrong (date never set), not a stale QR: trust the code instead.
  static const wrongClockSecs = 2 * 24 * 3600;

  /// null when [raw] is not an Aura pairing QR (wrong scheme, missing or malformed code).
  static PairQr? parse(String raw) {
    final u = Uri.tryParse(raw.trim());
    if (u == null || u.scheme != 'aura' || u.host != 'pair') return null;
    final clean = (u.queryParameters['c'] ?? '').toUpperCase().replaceAll(RegExp('[^A-Z0-9]'), '');
    if (clean.length != codeLength) return null;
    final b = StringBuffer();
    for (var i = 0; i < clean.length; i++) {
      if (i > 0 && i % 4 == 0) b.write(' ');
      b.write(clean[i]);
    }
    final q = u.queryParameters;
    return PairQr(b.toString(), int.tryParse(q['x'] ?? '') ?? 0, role: q['r'], name: q['n'], addr: q['a'], tailnet: q['t']);
  }

  bool expired(int nowSecs) => expiresAt > 0 && nowSecs > expiresAt + skewSecs && nowSecs - expiresAt < wrongClockSecs;

  /// Why [raw] cannot be used, or null when it can.
  static PairQrIssue? check(String raw, int nowSecs) {
    final q = parse(raw);
    if (q == null) return PairQrIssue.notAura;
    return q.expired(nowSecs) ? PairQrIssue.expired : null;
  }
}
