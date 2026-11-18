// The volume dial: turn a ring with one finger like the Galaxy Watch bezel. Every notch is felt
// (a crisp detent, no sound) and sent at once; turning past 0 or 100 hits a soft wall.
import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';

import 'fx.dart';

class HapticDial extends StatefulWidget {
  const HapticDial({
    super.key,
    required this.value,
    required this.onStep,
    this.max = 100,
    this.step = 2,
    this.size = 168,
    this.notchDegrees = 12,
    this.center,
  });

  /// Current level (0..max), or -1 while unknown.
  final int value;
  final int max;

  /// How much one notch changes the level (Windows moves 2% per volume key).
  final int step;

  /// Called once per notch: true = up (clockwise).
  final ValueChanged<bool> onStep;
  final double size;
  final double notchDegrees;
  final Widget? center;

  @override
  State<HapticDial> createState() => _HapticDialState();
}

class _HapticDialState extends State<HapticDial> {
  double? _lastAngle;
  double _carry = 0;   // degrees turned since the last notch
  int? _shown;         // optimistic level while turning; the PC's real value takes over after
  double _spin = 0;    // ring marker angle, follows the finger
  bool _wall = false;

  int get _level => _shown ?? widget.value;

  @override
  void didUpdateWidget(HapticDial old) {
    super.didUpdateWidget(old);
    if (_lastAngle == null) _shown = null;   // not touching: trust the PC
  }

  double _angle(Offset p) {
    final c = Offset(widget.size / 2, widget.size / 2);
    return math.atan2(p.dy - c.dy, p.dx - c.dx) * 180 / math.pi;
  }

  void _start(DragStartDetails d) {
    _lastAngle = _angle(d.localPosition);
    _carry = 0;
    _shown = widget.value < 0 ? null : widget.value;
    _wall = false;
  }

  void _update(DragUpdateDetails d) {
    final a = _angle(d.localPosition);
    var delta = a - (_lastAngle ?? a);
    if (delta > 180) delta -= 360;
    if (delta < -180) delta += 360;
    _lastAngle = a;
    _carry += delta;
    setState(() => _spin += delta);
    while (_carry.abs() >= widget.notchDegrees) {
      final up = _carry > 0;
      _carry -= up ? widget.notchDegrees : -widget.notchDegrees;
      final lvl = _level;
      if (lvl >= 0 && ((up && lvl >= widget.max) || (!up && lvl <= 0))) {
        if (!_wall) { _wall = true; Fx.edge(); }
        _carry = 0;
        return;
      }
      _wall = false;
      Fx.detent();
      if (lvl >= 0) setState(() => _shown = (lvl + (up ? widget.step : -widget.step)).clamp(0, widget.max));
      widget.onStep(up);
    }
  }

  void _end([DragEndDetails? _]) => _lastAngle = null;

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    final lvl = _level;
    return Semantics(
      label: 'Volume dial',
      value: lvl >= 0 ? '$lvl%' : null,
      increasedValue: lvl >= 0 ? '${(lvl + widget.step).clamp(0, widget.max)}%' : null,
      decreasedValue: lvl >= 0 ? '${(lvl - widget.step).clamp(0, widget.max)}%' : null,
      onIncrease: () { Fx.detent(); widget.onStep(true); },
      onDecrease: () { Fx.detent(); widget.onStep(false); },
      // An eager pan: the dial sits inside a ListView, and a plain pan lost the gesture fight to the
      // list's vertical drag, so most turns never started (no notches, no haptics).
      child: RawGestureDetector(
        key: const ValueKey('vol-dial'),
        behavior: HitTestBehavior.opaque,
        gestures: {
          _EagerPan: GestureRecognizerFactoryWithHandlers<_EagerPan>(
            _EagerPan.new,
            (g) => g
              ..onStart = _start
              ..onUpdate = _update
              ..onEnd = _end
              ..onCancel = _end,
          ),
        },
        child: SizedBox.square(
          dimension: widget.size,
          child: CustomPaint(
            painter: _DialPainter(
              fraction: lvl < 0 ? 0 : lvl / widget.max,
              spin: _spin,
              track: cs.primary.withValues(alpha: 0.14),
              fill: cs.primary,
              notch: cs.onSurface.withValues(alpha: 0.18),
            ),
            child: Center(child: widget.center),
          ),
        ),
      ),
    );
  }
}

/// A pan that claims the touch as soon as it lands, so a parent scroll view cannot steal it.
class _EagerPan extends PanGestureRecognizer {
  @override
  void addAllowedPointer(PointerDownEvent event) {
    super.addAllowedPointer(event);
    resolve(GestureDisposition.accepted);
  }
}

class _DialPainter extends CustomPainter {
  _DialPainter({required this.fraction, required this.spin, required this.track, required this.fill, required this.notch});
  final double fraction, spin;
  final Color track, fill, notch;

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final r = size.width / 2 - 10;
    const start = math.pi * 0.75, sweep = math.pi * 1.5;   // a 270° arc, open at the bottom
    final rect = Rect.fromCircle(center: c, radius: r);
    canvas.drawArc(rect, start, sweep, false, Paint()..color = track..style = PaintingStyle.stroke..strokeWidth = 10..strokeCap = StrokeCap.round);
    if (fraction > 0) {
      canvas.drawArc(rect, start, sweep * fraction, false, Paint()..color = fill..style = PaintingStyle.stroke..strokeWidth = 10..strokeCap = StrokeCap.round);
    }
    // bezel teeth that turn with the finger, so the ring visibly moves under it
    final tooth = Paint()..color = notch..strokeWidth = 2..strokeCap = StrokeCap.round;
    for (var i = 0; i < 30; i++) {
      final a = (i * 12 + spin) * math.pi / 180;
      final d = Offset(math.cos(a), math.sin(a));
      canvas.drawLine(c + d * (r - 20), c + d * (r - 13), tooth);
    }
  }

  @override
  bool shouldRepaint(_DialPainter o) => o.fraction != fraction || o.spin != spin || o.fill != fill;
}
