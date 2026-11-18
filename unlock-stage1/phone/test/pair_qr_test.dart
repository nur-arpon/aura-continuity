import 'package:flutter_test/flutter_test.dart';
import 'package:unlock_stage1/core/pair_qr.dart';

void main() {
  const code = 'GOLT-7MJ2-NJFB-3K6X-D5BU-MSNU-QALD-LH6L-BW';
  String qr({String c = code, int x = 1300}) => 'aura://pair?v=1&c=$c&x=$x';

  test('the PC QR parses into the same grouped code a user would type', () {
    final p = PairQr.parse(qr())!;
    expect(p.code, 'GOLT 7MJ2 NJFB 3K6X D5BU MSNU QALD LH6L BW');
    expect(p.expiresAt, 1300);
  });

  test('it expires after the PC time plus a small clock skew', () {
    final p = PairQr.parse(qr())!;
    expect(p.expired(1300), isFalse);
    expect(p.expired(1300 + PairQr.skewSecs), isFalse);
    expect(p.expired(1300 + PairQr.skewSecs + 1), isTrue);
    expect(PairQr.check(qr(), 5000), PairQrIssue.expired);
    expect(PairQr.check(qr(), 1000), isNull);
  });

  test('a phone whose clock is days off is not told the QR expired', () {
    final p = PairQr.parse(qr())!;
    expect(p.expired(1300 + PairQr.wrongClockSecs + 10), isFalse);
  });

  test('anything that is not an Aura pairing QR is refused', () {
    for (final bad in ['https://example.com', 'aura://other?c=$code', 'aura://pair?v=1', qr(c: 'ABCD'), 'GOLT7MJ2NJFB3K6XD5BUMSNUQALDLH6LBW', '']) {
      expect(PairQr.parse(bad), isNull, reason: bad);
      expect(PairQr.check(bad, 0), PairQrIssue.notAura);
    }
  });
}
