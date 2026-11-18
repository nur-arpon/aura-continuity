// Aura building blocks: the presence orb, cards, the big action tiles, pills, settings rows and
// the toast. Everything takes its colours from AuraColors, so light and dark come for free.
import 'dart:async';
import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'fx.dart';
import 'model.dart';
import 'motion.dart';
import 'theme.dart';

/// The Aura mark, alive: a soft field, two sensing rings, a bright core with an icon. [live]
/// adds a slow ripple (the PC is connected); breathing is always on unless the system asks
/// for reduced motion.
class AuraOrb extends StatefulWidget {
  const AuraOrb({super.key, this.size = 176, this.tone = Tone.info, this.icon, this.live = false});
  final double size;
  final Tone tone;
  final IconData? icon;
  final bool live;
  @override
  State<AuraOrb> createState() => _AuraOrbState();
}

class _AuraOrbState extends State<AuraOrb> with SingleTickerProviderStateMixin {
  late final AnimationController _c = AnimationController(vsync: this, duration: const Duration(milliseconds: 3200));

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final still = MediaQuery.maybeDisableAnimationsOf(context) ?? false;
    if (still) {
      _c.stop();
    } else if (!_c.isAnimating) {
      _c.repeat();
    }
  }

  @override
  void dispose() {
    _c.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final color = widget.tone == Tone.info ? a.glow : a.tone(widget.tone);
    return RepaintBoundary(
      child: SizedBox.square(
        dimension: widget.size,
        child: AnimatedBuilder(
          animation: _c,
          builder: (_, _) => CustomPaint(
            painter: _OrbPainter(t: _c.value, color: color, core: Colors.white, live: widget.live, dark: a.dark),
            child: Center(
              child: AnimatedSwitcher(
                duration: const Duration(milliseconds: 380),
                switchInCurve: Springs.curve,
                transitionBuilder: (c, an) => ScaleTransition(
                  scale: an,
                  child: FadeTransition(opacity: an, child: c),
                ),
                child: widget.icon == null
                    ? const SizedBox.shrink()
                    : Icon(
                        widget.icon,
                        key: ValueKey(widget.icon),
                        size: widget.size * 0.17,
                        color: a.dark ? const Color(0xFF0A1B3A) : Colors.white,
                      ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _OrbPainter extends CustomPainter {
  _OrbPainter({required this.t, required this.color, required this.core, required this.live, required this.dark});
  final double t;
  final Color color;
  final Color core;
  final bool live;
  final bool dark;

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    final r = size.width / 2;
    final breathe = 1 + 0.035 * math.sin(t * 2 * math.pi);
    // soft presence field
    canvas.drawCircle(
      c,
      r * 0.98,
      Paint()
        ..shader = RadialGradient(
          colors: [
            color.withValues(alpha: dark ? 0.42 : 0.30),
            color.withValues(alpha: dark ? 0.14 : 0.10),
            color.withValues(alpha: 0),
          ],
          stops: const [0, 0.55, 1],
        ).createShader(Rect.fromCircle(center: c, radius: r * 0.98)),
    );
    // ripple: the PC is connected right now
    if (live) {
      for (final k in [0.0, 0.5]) {
        final p = (t * 2 + k) % 1.0;
        canvas.drawCircle(
          c,
          r * (0.42 + 0.5 * p),
          Paint()
            ..style = PaintingStyle.stroke
            ..strokeWidth = 1.4
            ..color = color.withValues(alpha: 0.45 * (1 - p)),
        );
      }
    }
    // outer and inner sensing rings
    canvas.drawCircle(
      c,
      r * 0.80 * breathe,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = color.withValues(alpha: 0.32),
    );
    canvas.drawCircle(
      c,
      r * 0.56 * breathe,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2.2
        ..color = color.withValues(alpha: 0.85),
    );
    // bright core
    final coreR = r * 0.30;
    canvas.drawCircle(
      c,
      coreR + 6,
      Paint()
        ..color = color.withValues(alpha: 0.25)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 10),
    );
    canvas.drawCircle(
      c,
      coreR,
      Paint()
        ..shader = RadialGradient(
          center: const Alignment(-0.3, -0.35),
          // light: a deeper core so a white icon reads on it; dark: a bright core with a navy icon
          colors: dark ? [core, Color.lerp(core, color, 0.35)!, color] : [Color.lerp(core, color, 0.55)!, color, Color.lerp(color, Colors.black, 0.18)!],
          stops: const [0, 0.5, 1],
        ).createShader(Rect.fromCircle(center: c, radius: coreR)),
    );
  }

  @override
  bool shouldRepaint(_OrbPainter o) => o.t != t || o.color != color || o.live != live || o.dark != dark;
}

/// Page background: a deep gradient with a soft glow up top (navy in dark, sky in light).
class AuraBackground extends StatelessWidget {
  const AuraBackground({super.key, required this.child});
  final Widget child;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    if (a.expressive) return ColoredBox(color: a.bg, child: child);
    return DecoratedBox(
      decoration: BoxDecoration(
        gradient: LinearGradient(begin: Alignment.topCenter, end: Alignment.bottomCenter, colors: [a.bgTop, a.bg], stops: const [0, 0.42]),
      ),
      child: Stack(
        children: [
          Positioned(
            top: -140,
            right: -90,
            child: IgnorePointer(
              child: Container(
                width: 340,
                height: 340,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  gradient: RadialGradient(
                    colors: [
                      a.glow.withValues(alpha: a.dark ? 0.20 : 0.16),
                      a.glow.withValues(alpha: 0),
                    ],
                  ),
                ),
              ),
            ),
          ),
          Positioned.fill(child: child),
        ],
      ),
    );
  }
}

class AuraCard extends StatelessWidget {
  const AuraCard({super.key, required this.child, this.padding = const EdgeInsets.all(18), this.radius = 26, this.color});
  final Widget child;
  final EdgeInsets padding;
  final double radius;
  final Color? color;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    if (a.expressive) {
      // Expressive: a flat tonal container, no outline, bigger corners
      return Container(
        padding: padding,
        decoration: BoxDecoration(color: color ?? a.surface1, borderRadius: BorderRadius.circular(radius + 6)),
        child: child,
      );
    }
    return Container(
      padding: padding,
      decoration: BoxDecoration(
        color: color ?? a.surface1,
        borderRadius: BorderRadius.circular(radius),
        border: Border.all(color: a.dark ? a.divider.withValues(alpha: 0.7) : a.divider.withValues(alpha: 0.9)),
        boxShadow: a.dark ? null : [BoxShadow(color: const Color(0xFF0B1A33).withValues(alpha: 0.05), blurRadius: 18, offset: const Offset(0, 2))],
      ),
      child: child,
    );
  }
}

/// The big Lock PC / Unlock PC tiles.
class BigAction extends StatelessWidget {
  const BigAction({
    super.key,
    required this.icon,
    required this.label,
    required this.caption,
    required this.onTap,
    this.primary = false,
    this.busy = false,
    this.cue = Cue.tap,
    this.enabled = true,
  });
  final IconData icon;
  final String label;
  final String caption;
  final VoidCallback onTap;
  final bool primary;
  final bool busy;
  final Cue? cue;
  final bool enabled;

  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final fg = primary ? a.onPrimary : a.text;
    final sub = primary ? a.onPrimary.withValues(alpha: 0.78) : a.text3;
    return Pressable(
      onTap: busy ? null : onTap,
      cue: cue,
      enabled: enabled,
      semanticLabel: label,
      child: Container(
        height: 128,
        padding: const EdgeInsets.fromLTRB(16, 16, 14, 14),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(26),
          gradient: primary ? LinearGradient(begin: Alignment.topLeft, end: Alignment.bottomRight, colors: [a.primaryBright, a.primary]) : null,
          color: primary ? null : a.surface2,
          border: primary ? null : Border.all(color: a.divider),
          boxShadow: primary
              ? [
                  BoxShadow(
                    color: a.primary.withValues(alpha: a.dark ? 0.30 : 0.24),
                    blurRadius: 22,
                    offset: const Offset(0, 3),
                  ),
                ]
              : null,
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(
              width: 44,
              height: 44,
              decoration: BoxDecoration(shape: BoxShape.circle, color: primary ? Colors.white.withValues(alpha: 0.22) : a.primary.withValues(alpha: 0.14)),
              child: Center(
                child: AnimatedSwitcher(
                  duration: const Duration(milliseconds: 250),
                  child: busy
                      ? SizedBox.square(
                          key: const ValueKey('b'),
                          dimension: 20,
                          child: CircularProgressIndicator(strokeWidth: 2.4, color: fg),
                        )
                      : Icon(icon, key: const ValueKey('i'), color: primary ? fg : a.primary, size: 24),
                ),
              ),
            ),
            const Spacer(),
            Text(
              label,
              style: TextStyle(color: fg, fontSize: 19, fontWeight: FontWeight.w700, letterSpacing: -0.3),
            ),
            const SizedBox(height: 2),
            Text(
              caption,
              maxLines: 1,
              overflow: TextOverflow.ellipsis,
              style: TextStyle(color: sub, fontSize: 12.5, fontWeight: FontWeight.w500),
            ),
          ],
        ),
      ),
    );
  }
}

/// A small coloured capsule: dot + word (colour is never the only signal).
class Pill extends StatelessWidget {
  const Pill(this.text, {super.key, this.tone = Tone.info, this.icon});
  final String text;
  final Tone tone;
  final IconData? icon;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final c = a.tone(tone);
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
      decoration: BoxDecoration(
        color: c.withValues(alpha: a.dark ? 0.16 : 0.11),
        borderRadius: BorderRadius.circular(99),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (icon != null)
            Icon(icon, size: 14, color: c)
          else
            Container(
              width: 7,
              height: 7,
              decoration: BoxDecoration(color: c, shape: BoxShape.circle),
            ),
          const SizedBox(width: 6),
          Text(
            text,
            style: TextStyle(color: c, fontSize: 12.5, fontWeight: FontWeight.w600),
          ),
        ],
      ),
    );
  }
}

class SectionLabel extends StatelessWidget {
  const SectionLabel(this.text, {super.key, this.trailing});
  final String text;
  final Widget? trailing;
  @override
  Widget build(BuildContext context) {
    final th = Theme.of(context);
    final ex = context.aura.expressive;
    return Padding(
      padding: EdgeInsets.fromLTRB(ex ? 10 : 6, ex ? 28 : 26, 4, ex ? 12 : 10),
      child: Row(
        children: [
          Expanded(
            child: ex
                ? Text(text, style: th.textTheme.titleMedium?.copyWith(color: th.colorScheme.primary, fontWeight: FontWeight.w800))
                : Text(text.toUpperCase(), style: th.textTheme.labelSmall),
          ),
          ?trailing,
        ],
      ),
    );
  }
}

/// Rounded icon tile used at the start of rows.
class IconTile extends StatelessWidget {
  const IconTile(this.icon, {super.key, this.tone = Tone.info, this.size = 36});
  final IconData icon;
  final Tone tone;
  final double size;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final c = a.tone(tone);
    if (a.expressive) {
      final cs = Theme.of(context).colorScheme;
      final info = tone == Tone.info;
      return Container(
        width: size + 4,
        height: size + 4,
        decoration: BoxDecoration(
          color: info ? cs.primaryContainer : c.withValues(alpha: a.dark ? 0.22 : 0.16),
          borderRadius: BorderRadius.circular((size + 4) * 0.42),
        ),
        child: Icon(icon, size: size * 0.58, color: info ? cs.onPrimaryContainer : c),
      );
    }
    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        color: c.withValues(alpha: a.dark ? 0.16 : 0.11),
        borderRadius: BorderRadius.circular(size * 0.32),
      ),
      child: Icon(icon, size: size * 0.56, color: c),
    );
  }
}

/// A grouped list (settings style): rows separated by inset hairlines.
class Group extends StatelessWidget {
  const Group({super.key, required this.children});
  final List<Widget> children;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final out = <Widget>[];
    for (var i = 0; i < children.length; i++) {
      if (i > 0) {
        out.add(
          Padding(
            padding: const EdgeInsets.only(left: 62),
            child: Divider(color: a.divider.withValues(alpha: 0.8)),
          ),
        );
      }
      out.add(children[i]);
    }
    return AuraCard(
      padding: const EdgeInsets.symmetric(vertical: 4),
      radius: 22,
      child: Column(children: out),
    );
  }
}

class Row2 extends StatelessWidget {
  const Row2({
    super.key,
    required this.icon,
    required this.title,
    this.subtitle,
    this.trailing,
    this.onTap,
    this.tone = Tone.info,
    this.cue = Cue.tap,
    this.rowKey,
    this.onLongPress,
  });
  final IconData icon;
  final String title;
  final String? subtitle;
  final Widget? trailing;
  final VoidCallback? onTap;
  final VoidCallback? onLongPress;
  final Tone tone;
  final Cue? cue;
  final Key? rowKey;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final body = Padding(
      padding: const EdgeInsets.fromLTRB(14, 12, 12, 12),
      child: Row(
        children: [
          IconTile(icon, tone: tone),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  title,
                  style: TextStyle(color: a.text, fontSize: 15.5, fontWeight: FontWeight.w600),
                ),
                if (subtitle != null) ...[const SizedBox(height: 2), Text(subtitle!, style: TextStyle(color: a.text3, fontSize: 12.8, height: 1.35))],
              ],
            ),
          ),
          if (trailing != null) ...[const SizedBox(width: 8), trailing!],
        ],
      ),
    );
    if (onTap == null) return KeyedSubtree(key: rowKey, child: body);
    return Pressable(key: rowKey, onTap: onTap, onLongPress: onLongPress, cue: cue, scale: 0.98, child: body);
  }
}

/// A switch row that feels like one: toggle haptics both ways.
class SwitchRow extends StatelessWidget {
  const SwitchRow({
    super.key,
    required this.icon,
    required this.title,
    this.subtitle,
    required this.value,
    required this.onChanged,
    this.tone = Tone.info,
    this.switchKey,
  });
  final IconData icon;
  final String title;
  final String? subtitle;
  final bool value;
  final ValueChanged<bool> onChanged;
  final Tone tone;
  final Key? switchKey;
  @override
  Widget build(BuildContext context) => Row2(
    icon: icon,
    title: title,
    subtitle: subtitle,
    tone: tone,
    cue: null,
    onTap: () {
      Fx.toggle(!value);
      onChanged(!value);
    },
    trailing: Switch(
      key: switchKey,
      value: value,
      onChanged: (v) {
        Fx.toggle(v);
        onChanged(v);
      },
    ),
  );
}

/// Small text button with Aura styling (pill).
class PillButton extends StatelessWidget {
  const PillButton(this.label, {super.key, required this.onTap, this.filled = false, this.icon, this.cue = Cue.tap});
  final String label;
  final VoidCallback? onTap;
  final bool filled;
  final IconData? icon;
  final Cue? cue;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final cs = Theme.of(context).colorScheme;
    final bg = a.expressive ? (filled ? cs.primary : cs.secondaryContainer) : (filled ? a.primary : a.primary.withValues(alpha: a.dark ? 0.16 : 0.10));
    final fg = a.expressive ? (filled ? cs.onPrimary : cs.onSecondaryContainer) : (filled ? a.onPrimary : a.primary);
    return Pressable(
      onTap: onTap,
      cue: cue,
      semanticLabel: label,
      child: Container(
        padding: EdgeInsets.symmetric(horizontal: a.expressive ? 16 : 14, vertical: a.expressive ? 11 : 9),
        decoration: BoxDecoration(
          color: bg,
          borderRadius: BorderRadius.circular(99),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            if (icon != null) ...[Icon(icon, size: 17, color: fg), const SizedBox(width: 6)],
            Text(
              label,
              style: TextStyle(color: fg, fontSize: 14, fontWeight: a.expressive ? FontWeight.w700 : FontWeight.w600),
            ),
          ],
        ),
      ),
    );
  }
}

/// Full-width primary button (onboarding, dialogs).
class WideButton extends StatelessWidget {
  const WideButton(this.label, {super.key, required this.onTap, this.secondary = false, this.busy = false, this.cue = Cue.tap, this.icon});
  final String label;
  final VoidCallback? onTap;
  final bool secondary;
  final bool busy;
  final Cue? cue;
  final IconData? icon;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final cs = Theme.of(context).colorScheme;
    final fg = a.expressive ? (secondary ? cs.onSecondaryContainer : cs.onPrimary) : (secondary ? a.primary : a.onPrimary);
    return Pressable(
      onTap: busy ? null : onTap,
      enabled: onTap != null,
      cue: cue,
      semanticLabel: label,
      scale: 0.97,
      child: Container(
        height: 56,
        alignment: Alignment.center,
        decoration: a.expressive
            ? BoxDecoration(borderRadius: BorderRadius.circular(99), color: secondary ? cs.secondaryContainer : cs.primary)
            : BoxDecoration(
                borderRadius: BorderRadius.circular(18),
                gradient: secondary ? null : LinearGradient(colors: [a.primaryBright, a.primary]),
                color: secondary ? a.primary.withValues(alpha: a.dark ? 0.14 : 0.09) : null,
                boxShadow: secondary || onTap == null ? null : [BoxShadow(color: a.primary.withValues(alpha: 0.26), blurRadius: 18, offset: const Offset(0, 3))],
              ),
        child: busy
            ? SizedBox.square(dimension: 22, child: CircularProgressIndicator(strokeWidth: 2.5, color: fg))
            : Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  if (icon != null) ...[Icon(icon, color: fg, size: 20), const SizedBox(width: 8)],
                  Text(
                    label,
                    style: TextStyle(color: fg, fontSize: 16.5, fontWeight: FontWeight.w700),
                  ),
                ],
              ),
      ),
    );
  }
}

/// The toast: a pill that springs down from the top, says what happened, and leaves by itself.
class ToastHost extends StatefulWidget {
  const ToastHost({super.key, required this.child});
  final Widget child;
  @override
  State<ToastHost> createState() => _ToastHostState();
}

class _ToastHostState extends State<ToastHost> {
  AuraModel? _m;
  AuraToast? _t;
  Timer? _hide;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final m = AuraScope.read(context);
    if (m != _m) {
      _m?.toast.removeListener(_on);
      _m = m..toast.addListener(_on);
    }
  }

  void _on() {
    final t = _m!.toast.value;
    _hide?.cancel();
    setState(() => _t = t);
    if (t != null) _hide = Timer(Duration(milliseconds: t.busy ? 9000 : (t.action != null ? 4500 : 2600)), () => mounted ? setState(() => _t = null) : null);
  }

  @override
  void dispose() {
    _hide?.cancel();
    _m?.toast.removeListener(_on);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final top = MediaQuery.paddingOf(context).top + 8;
    final cs = Theme.of(context).colorScheme;
    final ex = a.expressive;
    final t = _t;
    return Stack(
      children: [
        widget.child,
        Positioned(
          top: top,
          left: 16,
          right: 16,
          child: AnimatedSwitcher(
            duration: const Duration(milliseconds: 420),
            reverseDuration: const Duration(milliseconds: 220),
            switchInCurve: Springs.curve,
            switchOutCurve: Curves.easeIn,
            transitionBuilder: (c, an) => FadeTransition(
              opacity: an,
              child: SlideTransition(
                position: Tween(begin: const Offset(0, -0.8), end: Offset.zero).animate(an),
                child: c,
              ),
            ),
            child: t == null
                ? const SizedBox.shrink()
                : Center(
                    key: ValueKey(t.id),
                    child: Material(
                      type: MaterialType.transparency,
                      child: GestureDetector(
                        onTap: () => setState(() => _t = null),
                        child: Container(
                          constraints: const BoxConstraints(maxWidth: 420),
                          padding: const EdgeInsets.fromLTRB(14, 10, 10, 10),
                          decoration: BoxDecoration(
                            color: a.expressive ? Theme.of(context).colorScheme.inverseSurface : (a.dark ? const Color(0xFF1B2F52) : const Color(0xFF0E1E3A)),
                            borderRadius: BorderRadius.circular(99),
                            boxShadow: [BoxShadow(color: Colors.black.withValues(alpha: 0.28), blurRadius: 24, offset: const Offset(0, 8))],
                          ),
                          child: Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              if (t.busy)
                                SizedBox.square(dimension: 16, child: CircularProgressIndicator(strokeWidth: 2, color: ex ? cs.inversePrimary : const Color(0xFF8EC5FF)))
                              else
                                Icon(t.icon ?? Icons.info_outline_rounded, size: 18, color: ex ? cs.inversePrimary : _toastColor(t.tone)),
                              const SizedBox(width: 10),
                              Flexible(
                                child: Text(
                                  t.text,
                                  maxLines: 2,
                                  overflow: TextOverflow.ellipsis,
                                  style: TextStyle(color: ex ? cs.onInverseSurface : const Color(0xFFF2F6FD), fontSize: 14, fontWeight: FontWeight.w600),
                                ),
                              ),
                              if (t.actionLabel != null) ...[
                                const SizedBox(width: 8),
                                Pressable(
                                  onTap: () {
                                    setState(() => _t = null);
                                    t.action?.call();
                                  },
                                  child: Container(
                                    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
                                    decoration: BoxDecoration(color: ex ? cs.inversePrimary : const Color(0xFF5AA2FF), borderRadius: BorderRadius.circular(99)),
                                    child: Text(
                                      t.actionLabel!,
                                      style: TextStyle(color: ex ? cs.inverseSurface : const Color(0xFF06142B), fontSize: 13, fontWeight: FontWeight.w700),
                                    ),
                                  ),
                                ),
                              ] else
                                const SizedBox(width: 6),
                            ],
                          ),
                        ),
                      ),
                    ),
                  ),
          ),
        ),
      ],
    );
  }

  Color _toastColor(Tone t) => switch (t) {
    Tone.good => const Color(0xFF4CD3B4),
    Tone.warn => const Color(0xFFF3B95F),
    _ => const Color(0xFF8EC5FF),
  };
}
