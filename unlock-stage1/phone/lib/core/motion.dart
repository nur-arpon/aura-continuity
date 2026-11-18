// Motion: springs everywhere, no linear tweens (same rules and constants as the owner's Spike app).
import 'package:flutter/material.dart';
import 'package:flutter/physics.dart';

import 'fx.dart';

abstract final class Springs {
  /// Press feedback: quick, a hint of overshoot.
  static const press = SpringDescription(mass: 1, stiffness: 520, damping: 26);

  /// Things landing into place (cards, chips): a visible bounce.
  static const bouncy = SpringDescription(mass: 1, stiffness: 300, damping: 17);

  /// Large movements (pages, sheets): no wobble, fast settle.
  static const smooth = SpringDescription(mass: 1, stiffness: 240, damping: 30);

  static const Curve curve = SpringCurve(bouncy);
  static const Curve smoothCurve = SpringCurve(smooth);
}

class SpringCurve extends Curve {
  const SpringCurve(this.spring);
  final SpringDescription spring;
  @override
  double transformInternal(double t) => SpringSimulation(spring, 0, 1, 0).x(t * 0.9);
}

/// A child that squishes under the finger and springs back, with a haptic + sound on the tap.
/// Every button in Aura is built on this.
class Pressable extends StatefulWidget {
  const Pressable({
    super.key,
    required this.child,
    this.onTap,
    this.onLongPress,
    this.scale = 0.95,
    this.cue = Cue.tap,
    this.enabled = true,
    this.semanticLabel,
    this.radius = 20,
  });
  final Widget child;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;
  final double scale;

  /// null = no feedback here (the action gives its own, e.g. a native activity).
  final Cue? cue;
  final bool enabled;
  final String? semanticLabel;
  final double radius;

  @override
  State<Pressable> createState() => _PressableState();
}

class _PressableState extends State<Pressable> with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController.unbounded(vsync: this, value: 1);

  void _to(double target, {double velocity = 0}) => _c.animateWith(SpringSimulation(Springs.press, _c.value, target, velocity));

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final on = widget.enabled && (widget.onTap != null || widget.onLongPress != null);
    return Semantics(
      button: true,
      enabled: on,
      label: widget.semanticLabel,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTapDown: on ? (_) => _to(widget.scale) : null,
        onTapUp: on
            ? (_) {
                _to(1, velocity: 4);
                if (widget.cue != null) Fx.play(widget.cue!);
                widget.onTap?.call();
              }
            : null,
        onTapCancel: on ? () => _to(1) : null,
        onLongPress: on && widget.onLongPress != null
            ? () {
                Fx.play(Cue.toggleOn);
                _to(1, velocity: 6);
                widget.onLongPress!();
              }
            : null,
        child: AnimatedBuilder(
          animation: _c,
          builder: (_, child) => Transform.scale(scale: _c.value, child: child),
          child: AnimatedOpacity(duration: const Duration(milliseconds: 200), opacity: widget.enabled ? 1 : 0.45, child: widget.child),
        ),
      ),
    );
  }
}

/// Fades + rises + springs its child in when first built. [index] staggers siblings 40 ms apart.
class Entrance extends StatefulWidget {
  const Entrance({super.key, required this.child, this.index = 0, this.offset = 18});
  final Widget child;
  final int index;
  final double offset;
  @override
  State<Entrance> createState() => _EntranceState();
}

class _EntranceState extends State<Entrance> with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController.unbounded(vsync: this, value: 0);

  @override
  void initState() {
    super.initState();
    Future<void>.delayed(Duration(milliseconds: 40 * widget.index.clamp(0, 12)), () {
      if (mounted) _c.animateWith(SpringSimulation(Springs.bouncy, 0, 1, 0));
    });
  }

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
    animation: _c,
    builder: (_, child) => Opacity(
      opacity: _c.value.clamp(0.0, 1.0),
      child: Transform.translate(offset: Offset(0, (1 - _c.value) * widget.offset), child: child),
    ),
    child: widget.child,
  );
}

/// Pages slide up a little and fade in on a spring (Apple-style), and back the same way.
Route<T> auraRoute<T>(Widget page) => PageRouteBuilder<T>(
  transitionDuration: const Duration(milliseconds: 420),
  reverseTransitionDuration: const Duration(milliseconds: 300),
  pageBuilder: (_, _, _) => page,
  transitionsBuilder: (_, anim, sec, child) {
    final a = CurvedAnimation(parent: anim, curve: Springs.smoothCurve, reverseCurve: Curves.easeInCubic);
    final back = CurvedAnimation(parent: sec, curve: Curves.easeOutCubic);
    return FadeTransition(
      opacity: a,
      child: SlideTransition(
        position: Tween(begin: const Offset(0, 0.06), end: Offset.zero).animate(a),
        child: ScaleTransition(scale: Tween(begin: 1.0, end: 0.96).animate(back), child: child),
      ),
    );
  },
);
