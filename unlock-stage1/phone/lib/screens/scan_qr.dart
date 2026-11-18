// "Scan QR": the camera reads the PC's Add-a-phone QR and hands the pairing code back. Typing the
// code stays the fallback (the camera is asked for only here, the moment the user taps Scan).
import 'package:flutter/material.dart';
import 'package:flutter_animate/flutter_animate.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import '../core/fx.dart';
import '../core/motion.dart';
import '../core/pair_qr.dart';
import '../core/theme.dart';

/// Opens the scanner with a spring slide-up; resolves to the grouped code or null (cancelled / typed instead).
/// [hint] is the line under the viewfinder (the pairing screen says which PC button makes the right QR).
Future<String?> scanPairQr(BuildContext context, {String hint = ScanQrPage.defaultHint}) => Navigator.of(context).push<String>(
  PageRouteBuilder<String>(
    transitionDuration: const Duration(milliseconds: 520),
    reverseTransitionDuration: const Duration(milliseconds: 260),
    pageBuilder: (_, _, _) => ScanQrPage(hint: hint),
    transitionsBuilder: (_, an, _, child) {
      final c = CurvedAnimation(parent: an, curve: Springs.smoothCurve, reverseCurve: Curves.easeInCubic);
      return FadeTransition(
        opacity: an,
        child: SlideTransition(position: Tween(begin: const Offset(0, 0.12), end: Offset.zero).animate(c), child: child),
      );
    },
  ),
);

class ScanQrPage extends StatefulWidget {
  const ScanQrPage({super.key, this.hint = defaultHint});
  final String hint;
  static const defaultHint = 'On your PC open Aura, Devices, then Add another phone. Point the camera at the QR code.';
  @override
  State<ScanQrPage> createState() => _ScanQrPageState();
}

class _ScanQrPageState extends State<ScanQrPage> {
  final _ctl = MobileScannerController(formats: const [BarcodeFormat.qrCode], detectionSpeed: DetectionSpeed.noDuplicates);
  String? _msg;
  bool _done = false;

  @override
  void dispose() {
    _ctl.dispose();
    super.dispose();
  }

  void _onDetect(BarcodeCapture cap) {
    if (_done) return;
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    for (final b in cap.barcodes) {
      final raw = b.rawValue;
      if (raw == null) continue;
      final issue = PairQr.check(raw, now);
      if (issue == null) {
        _done = true;
        Fx.success();
        PairQr.last = PairQr.parse(raw);
        Navigator.of(context).pop(PairQr.parse(raw)!.code);
        return;
      }
      setState(() => _msg = issue == PairQrIssue.expired
          ? 'This QR expired. On your PC choose Add a phone again.'
          : 'That is not an Aura pairing QR.');
      Fx.error();
      return;
    }
  }

  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return Scaffold(
      backgroundColor: Colors.black,
      body: Stack(
        fit: StackFit.expand,
        children: [
          MobileScanner(
            controller: _ctl,
            onDetect: _onDetect,
            errorBuilder: (_, e) => Center(
              child: Padding(
                padding: const EdgeInsets.all(32),
                child: Text(
                  e.errorCode == MobileScannerErrorCode.permissionDenied
                      ? 'Camera access is off. Allow it for Aura in Settings, or type the code instead.'
                      : 'The camera is not available. Type the code instead.',
                  textAlign: TextAlign.center,
                  style: const TextStyle(color: Colors.white, fontSize: 16, height: 1.4),
                ),
              ),
            ),
          ),
          // viewfinder: a rounded frame that breathes with the spring curve
          Center(
            child: Container(
              width: 250,
              height: 250,
              decoration: BoxDecoration(
                border: Border.all(color: a.primaryBright, width: 3),
                borderRadius: BorderRadius.circular(28),
              ),
            ).animate(onPlay: (c) => c.repeat(reverse: true)).scale(
              begin: const Offset(0.96, 0.96),
              end: const Offset(1.03, 1.03),
              duration: 1400.ms,
              curve: Springs.smoothCurve,
            ),
          ),
          SafeArea(
            child: Column(
              children: [
                Align(
                  alignment: Alignment.centerLeft,
                  child: IconButton(
                    key: const ValueKey('scan-close'),
                    onPressed: () { Fx.tap(); Navigator.of(context).pop(); },
                    icon: const Icon(Icons.close_rounded, color: Colors.white),
                  ),
                ),
                const Spacer(),
                AnimatedSwitcher(
                  duration: const Duration(milliseconds: 300),
                  child: Padding(
                    key: ValueKey(_msg),
                    padding: const EdgeInsets.symmetric(horizontal: 28),
                    child: Text(
                      _msg ?? widget.hint,
                      textAlign: TextAlign.center,
                      style: TextStyle(color: _msg == null ? Colors.white : a.warning, fontSize: 15, fontWeight: FontWeight.w600, height: 1.4),
                    ),
                  ),
                ),
                const SizedBox(height: 18),
                TextButton(
                  key: const ValueKey('scan-type'),
                  onPressed: () { Fx.tap(); Navigator.of(context).pop(); },
                  child: const Text('Type the code instead', style: TextStyle(color: Colors.white70)),
                ),
                const SizedBox(height: 16),
              ],
            ),
          ),
        ],
      ),
    );
  }
}
