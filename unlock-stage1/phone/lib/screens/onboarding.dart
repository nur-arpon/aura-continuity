// First run: Welcome -> Pair (scan the PC's QR, or type the code) -> Allow everything (one tap walks
// every permission, live checklist) -> Connect your watch -> Done. Every step can go back, every optional step can be
// skipped, and nothing is asked before the screen that explains why.
import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_animate/flutter_animate.dart';

import '../core/fx.dart';
import '../core/model.dart';
import '../core/motion.dart';
import '../core/setup.dart';
import '../core/theme.dart';
import '../core/widgets.dart';
import '../core/pair_qr.dart';
import 'scan_qr.dart';

/// Upper-cases, drops anything that is not a letter or digit, and groups by 4: "GOLT 7MJ2 ...".
class PairCodeFormatter extends TextInputFormatter {
  static const length = 34;
  static String clean(String s) => s.toUpperCase().replaceAll(RegExp('[^A-Z0-9]'), '');
  static String group(String raw) {
    final b = StringBuffer();
    for (var i = 0; i < raw.length; i++) {
      if (i > 0 && i % 4 == 0) b.write(' ');
      b.write(raw[i]);
    }
    return b.toString();
  }

  @override
  TextEditingValue formatEditUpdate(TextEditingValue oldValue, TextEditingValue newValue) {
    final raw = clean(newValue.text);
    final t = group(raw.length > length ? raw.substring(0, length) : raw);
    return TextEditingValue(
      text: t,
      selection: TextSelection.collapsed(offset: t.length),
    );
  }
}

class Onboarding extends StatefulWidget {
  const Onboarding({super.key, required this.onDone, this.initialStep = 0});
  final VoidCallback onDone;
  final int initialStep;
  @override
  State<Onboarding> createState() => _OnboardingState();
}

class _OnboardingState extends State<Onboarding> {
  late int _step = widget.initialStep;
  final _code = TextEditingController();
  String? _err;

  /// null until the user says which phone this is: 'main' (unlocks the PC) or 'extra' (share / control only).
  String? _role;
  bool _busy = false;
  int _shake = 0;
  SetupController? _setup;
  Timer? _watchTimer;

  SetupController _sc(AuraModel m) => _setup ??= SetupController(m.bridge);

  @override
  void dispose() {
    _watchTimer?.cancel();
    _setup?.dispose();
    _code.dispose();
    super.dispose();
  }

  void _go(int s) {
    FocusScope.of(context).unfocus();
    setState(() => _step = s);
    final sc = _sc(AuraScope.read(context));
    if (s == 2) unawaited(sc.refresh());
    // the watch step re-checks every few seconds while it is on screen (Data Layer, no background work)
    _watchTimer?.cancel();
    if (s == 3) {
      unawaited(sc.refreshWatch());
      _watchTimer = Timer.periodic(const Duration(seconds: 4), (_) => sc.refreshWatch());
    }
  }

  Future<void> _scan(AuraModel m) async {
    Fx.tap();
    PairQr.last = null;
    final code = await scanPairQr(
      context,
      hint: _role == 'extra'
          ? 'On your PC open Aura, Devices, then Add another phone. Point the camera at the QR code.'
          : 'On your PC open Aura, Devices, then Replace main phone. Point the camera at the QR code.',
    );
    if (code == null || !mounted) return;
    if (!await _roleMatchesQr(PairQr.last)) return;
    if (!mounted) return;
    _code.value = PairCodeFormatter().formatEditUpdate(_code.value, TextEditingValue(text: code));
    await _pair(m);
  }

  Future<void> _pair(AuraModel m) async {
    setState(() {
      _busy = true;
      _err = null;
    });
    final ok = await m.pair(_code.text, role: _role ?? 'main');
    if (!mounted) return;
    setState(() {
      _busy = false;
      if (!ok) {
        _err = 'That code is not right. Check it on your PC and try again.';
        _shake++;
      }
    });
    if (ok) _go(2);
  }

  /// The QR says which PC button made it. If that is not the phone type the user picked, say so before
  /// anything is paired (pairing as the wrong kind replaces the main phone or hides Lock / Unlock).
  Future<bool> _roleMatchesQr(PairQr? q) async {
    if (q == null) return true;
    final isExtraQr = q.role == 'companion';
    if (_role == 'extra' && isExtraQr) return true;
    if (_role == 'main' && !isExtraQr) return true;
    final a = context.aura;
    final bool? useQr = await showDialog<bool>(
      context: context,
      builder: (c) => AlertDialog(
        icon: Icon(isExtraQr ? Icons.phone_android_rounded : Icons.smartphone_rounded, color: a.warning),
        title: Text(isExtraQr ? 'That QR is for an extra phone' : 'That QR would make this your main phone'),
        content: Text(
          isExtraQr
              ? 'You chose "My main phone", but this QR came from "Add another phone" on your PC. Use this phone as an extra phone instead?'
              : 'You chose "An extra phone", but this QR came from "Replace main phone" on your PC. That would make this phone the one that unlocks your PC and replace your current main phone.\n\nFor an extra phone, open Aura › Devices › Add another phone on your PC and scan that QR.',
        ),
        actions: [
          TextButton(key: const ValueKey('qr-mismatch-no'), onPressed: () => Navigator.pop(c, false), child: Text(isExtraQr ? 'Cancel' : 'Scan a different QR')),
          TextButton(
            key: const ValueKey('qr-mismatch-yes'),
            onPressed: () => Navigator.pop(c, true),
            child: Text(isExtraQr ? 'Use as extra phone' : 'Make this my main phone', style: TextStyle(color: isExtraQr ? a.primary : a.danger, fontWeight: FontWeight.w700)),
          ),
        ],
      ),
    );
    if (useQr != true) return false;
    setState(() => _role = isExtraQr ? 'extra' : 'main');
    return true;
  }

  Widget _roleCard(String key, IconData icon, String title, String body, String pcButton) {
    final a = context.aura;
    final on = _role == key;
    return GestureDetector(
      key: ValueKey('role-$key'),
      behavior: HitTestBehavior.opaque,
      onTap: () {
        Fx.tick();
        setState(() {
          _role = key;
          _err = null;
        });
      },
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 250),
        curve: Springs.smoothCurve,
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(18),
          color: on ? a.primary.withValues(alpha: 0.14) : a.surface3.withValues(alpha: 0.45),
          border: Border.all(color: on ? a.primary : a.border.withValues(alpha: 0.35), width: on ? 2 : 1),
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Icon(on ? Icons.radio_button_checked_rounded : Icons.radio_button_unchecked_rounded, color: on ? a.primary : a.text3, size: 22),
            const SizedBox(width: 12),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Row(children: [
                    Icon(icon, size: 18, color: a.text),
                    const SizedBox(width: 6),
                    Text(title, style: TextStyle(color: a.text, fontSize: 16, fontWeight: FontWeight.w700)),
                  ]),
                  const SizedBox(height: 3),
                  Text(body, style: TextStyle(color: a.text2, fontSize: 13, height: 1.35)),
                  const SizedBox(height: 5),
                  Text.rich(TextSpan(style: TextStyle(color: a.text3, fontSize: 12.5, height: 1.35), children: [
                    const TextSpan(text: 'On the PC: Aura › Devices › '),
                    TextSpan(text: pcButton, style: TextStyle(color: a.text, fontWeight: FontWeight.w700)),
                  ])),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }

  Future<void> _paste() async {
    Fx.tap();
    final d = await Clipboard.getData('text/plain');
    final t = d?.text ?? '';
    if (t.isEmpty) return;
    _code.value = PairCodeFormatter().formatEditUpdate(_code.value, TextEditingValue(text: t));
    setState(() => _err = null);
  }

  @override
  Widget build(BuildContext context) {
    final m = AuraScope.of(context);
    final pages = [_welcome(), _pairStep(m), _ready(m), _watch(m), _done()];
    return Scaffold(
      resizeToAvoidBottomInset: true,
      body: AuraBackground(
        child: SafeArea(
          child: Column(
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(12, 8, 12, 0),
                child: Row(
                  children: [
                    AnimatedOpacity(
                      duration: const Duration(milliseconds: 200),
                      opacity: _step == 1 ? 1 : 0,
                      child: IgnorePointer(
                        ignoring: _step != 1,
                        child: IconButton(
                          key: const ValueKey('ob-back'),
                          onPressed: () { Fx.tick(); _go(0); },
                          icon: Icon(Icons.arrow_back_rounded, color: context.aura.text2),
                        ),
                      ),
                    ),
                    const Spacer(),
                    _Dots(step: _step, count: pages.length),
                    const Spacer(),
                    const SizedBox(width: 48),
                  ],
                ),
              ),
              Expanded(
                child: AnimatedSwitcher(
                  duration: const Duration(milliseconds: 480),
                  switchInCurve: Springs.smoothCurve,
                  switchOutCurve: Curves.easeInCubic,
                  transitionBuilder: (c, an) => FadeTransition(
                    opacity: an,
                    child: SlideTransition(
                      position: Tween(begin: const Offset(0.08, 0), end: Offset.zero).animate(an),
                      child: c,
                    ),
                  ),
                  child: KeyedSubtree(key: ValueKey('step$_step'), child: pages[_step]),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _scaffold({required List<Widget> top, required List<Widget> bottom}) => LayoutBuilder(
    builder: (context, c) => SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(24, 8, 24, 20),
      child: ConstrainedBox(
        constraints: BoxConstraints(minHeight: c.maxHeight - 28),
        child: IntrinsicHeight(
          child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: [...top, const Spacer(), ...bottom]),
        ),
      ),
    ),
  );

  Widget _welcome() {
    final a = context.aura;
    return _scaffold(
      top: [
        const SizedBox(height: 28),
        const Center(child: AuraOrb(size: 220, live: true)),
        const SizedBox(height: 18),
        Text(
          'Aura',
          textAlign: TextAlign.center,
          style: TextStyle(color: a.text, fontSize: 44, fontWeight: FontWeight.w800, letterSpacing: -1.6),
        ).animate().fadeIn(duration: 500.ms, delay: 120.ms).moveY(begin: 12, end: 0, curve: Springs.curve, duration: 700.ms),
        Text(
          'Continuity for your PC',
          textAlign: TextAlign.center,
          style: TextStyle(color: a.text2, fontSize: 17, fontWeight: FontWeight.w500),
        ).animate().fadeIn(duration: 500.ms, delay: 220.ms),
        const SizedBox(height: 30),
        for (final (n, f) in const [
          (Icons.fingerprint_rounded, ('Unlock with your phone', 'Your PC opens when you sit down. No password.')),
          (Icons.cast_rounded, ('Pick up where you left off', 'Videos at the exact second, tabs, files, clipboard.')),
          (Icons.public_rounded, ('From anywhere', 'Lock your PC or carry on from the train.')),
        ].indexed)
          Padding(
            padding: const EdgeInsets.only(bottom: 14),
            child: Row(
              children: [
                IconTile(f.$1, size: 40),
                const SizedBox(width: 14),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        f.$2.$1,
                        style: TextStyle(color: a.text, fontSize: 15.5, fontWeight: FontWeight.w700),
                      ),
                      Text(f.$2.$2, style: TextStyle(color: a.text3, fontSize: 13, height: 1.35)),
                    ],
                  ),
                ),
              ],
            ).animate().fadeIn(delay: (320 + 90 * n).ms, duration: 450.ms).moveX(begin: 16, end: 0, curve: Springs.curve, duration: 650.ms),
          ),
      ],
      bottom: [
        const SizedBox(height: 20),
        WideButton('Get started', key: const ValueKey('ob-start'), onTap: () => _go(1)),
        const SizedBox(height: 10),
        Text(
          'Only between your own devices. End-to-end encrypted.',
          textAlign: TextAlign.center,
          style: TextStyle(color: a.text3, fontSize: 12),
        ),
      ],
    );
  }

  Widget _pairStep(AuraModel m) {
    final a = context.aura;
    final n = PairCodeFormatter.clean(_code.text).length;
    final full = n == PairCodeFormatter.length;
    return _scaffold(
      top: [
        const SizedBox(height: 8),
        const Align(alignment: Alignment.centerLeft, child: IconTile(Icons.laptop_windows_rounded, size: 56)),
        const SizedBox(height: 18),
        Text(
          'Which phone is this?',
          style: TextStyle(color: a.text, fontSize: 30, fontWeight: FontWeight.w800, letterSpacing: -0.8),
        ),
        const SizedBox(height: 8),
        Text(
          'Pick one, then scan the QR code your PC shows for it. You only do this once.',
          style: TextStyle(color: a.text2, fontSize: 15, height: 1.4),
        ),
        const SizedBox(height: 14),
        _roleCard('main', Icons.lock_open_rounded, 'My main phone', 'Unlocks and locks your PC, and does everything else. Replaces the main phone your PC has now, if any.', 'Replace main phone…'),
        const SizedBox(height: 10),
        _roleCard('extra', Icons.phone_android_rounded, 'An extra phone', 'Share files, continue things and control your PC. It never unlocks or locks it, and your main phone stays as it is.', 'Add another phone…'),
        const SizedBox(height: 22),
        WideButton('Scan QR code', key: const ValueKey('ob-scan'), icon: Icons.qr_code_scanner_rounded, busy: _busy, onTap: _role == null ? null : () => _scan(m))
            .animate()
            .fadeIn(duration: 350.ms)
            .moveY(begin: 10, end: 0, curve: Springs.curve, duration: 600.ms),
        const SizedBox(height: 16),
        Center(child: Text(_role == null ? 'Pick a phone type above to continue' : 'or type the code', style: TextStyle(color: a.text3, fontSize: 12.5, fontWeight: FontWeight.w600))),
        const SizedBox(height: 12),
        AuraCard(
          padding: const EdgeInsets.fromLTRB(16, 12, 12, 12),
          radius: 20,
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Text(
                    'PAIRING CODE',
                    style: TextStyle(color: a.text3, fontSize: 11, fontWeight: FontWeight.w700, letterSpacing: 1.1),
                  ),
                  const Spacer(),
                  Text(
                    '$n / ${PairCodeFormatter.length}',
                    style: TextStyle(color: _err != null ? a.warning : (full ? a.success : a.text3), fontSize: 12, fontWeight: FontWeight.w600),
                  ),
                  const SizedBox(width: 10),
                  PillButton('Paste', key: const ValueKey('ob-paste'), icon: Icons.content_paste_rounded, cue: null, onTap: _paste),
                ],
              ),
              TextField(
                key: const ValueKey('ob-code'),
                controller: _code,
                inputFormatters: [PairCodeFormatter()],
                textCapitalization: TextCapitalization.characters,
                autocorrect: false,
                enableSuggestions: false,
                keyboardType: TextInputType.visiblePassword,
                maxLines: 3,
                minLines: 2,
                onChanged: (_) => setState(() => _err = null),
                style: TextStyle(color: a.text, fontSize: 22, fontFamily: 'monospace', fontWeight: FontWeight.w600, letterSpacing: 1.5, height: 1.4),
                decoration: InputDecoration(
                  border: InputBorder.none,
                  hintText: 'XXXX XXXX XXXX XXXX …',
                  hintStyle: TextStyle(color: a.text3.withValues(alpha: 0.5), fontFamily: 'monospace', fontSize: 22, letterSpacing: 1.5),
                ),
              ),
              ClipRRect(
                borderRadius: BorderRadius.circular(4),
                child: TweenAnimationBuilder<double>(
                  tween: Tween(end: n / PairCodeFormatter.length),
                  duration: const Duration(milliseconds: 350),
                  curve: Springs.smoothCurve,
                  builder: (_, v, _) => LinearProgressIndicator(
                    value: v,
                    minHeight: 4,
                    color: _err != null ? a.warning : (full ? a.success : a.primary),
                    backgroundColor: a.surface3,
                  ),
                ),
              ),
            ],
          ),
        ).animate(key: ValueKey(_shake), target: _shake > 0 ? 1 : 0).shakeX(hz: 6, amount: 7, duration: 420.ms),
        AnimatedSize(
          duration: const Duration(milliseconds: 250),
          curve: Springs.smoothCurve,
          child: _err == null
              ? const SizedBox(width: double.infinity)
              : Padding(
                  padding: const EdgeInsets.only(top: 12, left: 4),
                  child: Row(
                    children: [
                      Icon(Icons.error_outline_rounded, color: a.warning, size: 18),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          _err!,
                          style: TextStyle(color: a.warning, fontSize: 13.5, fontWeight: FontWeight.w600),
                        ),
                      ),
                    ],
                  ),
                ),
        ),
      ],
      bottom: [
        const SizedBox(height: 20),
        WideButton('Pair', key: const ValueKey('ob-pair'), busy: _busy, cue: Cue.tap, onTap: full && _role != null ? () => _pair(m) : null),
        const SizedBox(height: 10),
        Text(
          'Letters O and I may be typed as 0 and 1.',
          textAlign: TextAlign.center,
          style: TextStyle(color: a.text3, fontSize: 12),
        ),
      ],
    );
  }

  Widget _ready(AuraModel m) {
    final a = context.aura;
    final sc = _sc(m);
    final linked = m.s['companion'] == true;
    final canLink = m.s['companionSupported'] == true;
    return ListenableBuilder(
      listenable: sc,
      builder: (context, _) => _scaffold(
        top: [
          const SizedBox(height: 8),
          Center(
            child: AuraOrb(
              size: 120,
              tone: sc.allGranted ? Tone.good : Tone.info,
              icon: sc.allGranted ? Icons.check_rounded : Icons.shield_moon_rounded,
              live: true,
            ).animate().scale(begin: const Offset(0.6, 0.6), curve: Springs.curve, duration: 700.ms),
          ),
          const SizedBox(height: 12),
          Text(
            'Paired with ${m.pcName == 'Your PC' ? 'your PC' : m.pcName}',
            textAlign: TextAlign.center,
            style: TextStyle(color: a.text, fontSize: 26, fontWeight: FontWeight.w800, letterSpacing: -0.7),
          ),
          const SizedBox(height: 6),
          Text(
            'One tap, then Android opens each page in turn. Switch Aura on and come back; Aura carries on by itself.',
            textAlign: TextAlign.center,
            style: TextStyle(color: a.text2, fontSize: 14.5, height: 1.4),
          ),
          const SizedBox(height: 18),
          if (sc.items.isNotEmpty) ...[
            Row(
              children: [
                Text('${sc.grantedCount} of ${sc.items.length} allowed', key: const ValueKey('ob-count'), style: TextStyle(color: a.text2, fontSize: 13, fontWeight: FontWeight.w700)),
                const SizedBox(width: 12),
                Expanded(
                  child: ClipRRect(
                    borderRadius: BorderRadius.circular(4),
                    child: TweenAnimationBuilder<double>(
                      tween: Tween(end: sc.grantedCount / sc.items.length),
                      duration: const Duration(milliseconds: 600),
                      curve: Springs.smoothCurve,
                      builder: (_, v, _) => LinearProgressIndicator(value: v, minHeight: 5, color: sc.allGranted ? a.success : a.primary, backgroundColor: a.surface3),
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 10),
          ],
          Group(
            children: [
              for (final it in sc.items) _permRow(sc, it),
              if (canLink)
                Row2(
                  rowKey: const ValueKey('ob-link'),
                  icon: Icons.bolt_rounded,
                  tone: linked ? Tone.good : Tone.info,
                  title: 'Always ready',
                  subtitle: 'Link your PC once in the Android sheet. Aura then starts by itself, with no polling.',
                  trailing: linked ? Icon(Icons.check_circle_rounded, color: a.success) : PillButton('Link', cue: null, onTap: m.linkCompanion),
                  onTap: linked ? null : m.linkCompanion,
                ),
            ],
          ),
        ],
        bottom: [
          const SizedBox(height: 20),
          if (!sc.allGranted)
            WideButton('Allow everything', key: const ValueKey('ob-perms'), icon: Icons.verified_user_rounded, busy: sc.walking, onTap: sc.allowEverything)
          else
            WideButton('Continue', key: const ValueKey('ob-continue'), onTap: () => _go(3)),
          const SizedBox(height: 6),
          if (!sc.allGranted)
            TextButton(
              key: const ValueKey('ob-later'),
              onPressed: sc.walking
                  ? null
                  : () {
                      Fx.tap();
                      _go(3);
                    },
              child: Text('Do this later in Settings', style: TextStyle(color: a.text3)),
            ),
        ],
      ),
    );
  }

  Widget _permRow(SetupController sc, SetupItem it) {
    final a = context.aura;
    final meta = setupMeta[it.id] ?? const SetupMeta(Icons.check_rounded, 'Permission', '');
    final active = sc.current == it.id;
    final declined = !it.granted && !sc.walking && sc.skipped.contains(it.id);
    return Row2(
      rowKey: ValueKey('perm-${it.id}'),
      icon: meta.icon,
      tone: it.granted ? Tone.good : (declined ? Tone.warn : Tone.info),
      title: meta.title,
      subtitle: declined ? 'Not allowed yet. Tap to try again.' : meta.why,
      trailing: AnimatedSwitcher(
        duration: const Duration(milliseconds: 420),
        switchInCurve: Springs.curve,
        transitionBuilder: (c, an) => ScaleTransition(scale: an, child: c),
        child: it.granted
            ? Icon(Icons.check_circle_rounded, key: const ValueKey('ok'), color: a.success)
            : active
            ? SizedBox(key: const ValueKey('busy'), width: 22, height: 22, child: CircularProgressIndicator(strokeWidth: 2.4, color: a.primary))
            : Icon(Icons.radio_button_unchecked_rounded, key: const ValueKey('todo'), color: a.text3),
      ),
      onTap: it.granted || sc.walking ? null : () => sc.requestOne(it.id),
    );
  }

  Widget _watch(AuraModel m) {
    final a = context.aura;
    final sc = _sc(m);
    return ListenableBuilder(
      listenable: sc,
      builder: (context, _) {
        final st = sc.watchState;
        final (icon, tone, title, body, cta) = switch (st) {
          'connected' => (Icons.watch_rounded, Tone.good, '${sc.watchName.isEmpty ? 'Your watch' : sc.watchName} is connected', 'Lock and unlock your PC from your wrist.', null),
          'unpaired' => (Icons.watch_rounded, Tone.info, 'Aura is on your watch', 'One last step on the PC: open Aura, Devices, Pair a watch, then open Aura on the watch and tap Pair with PC.', null),
          'away' => (Icons.watch_off_rounded, Tone.warn, 'Watch paired, but out of reach', 'Wake the watch and keep it near this phone. Aura checks again by itself.', null),
          'app-missing' => (Icons.get_app_rounded, Tone.info, 'Install Aura on your watch', 'Your watch is connected, but the Aura watch app is not on it yet. The Play Store page has an Install on watch button.', 'Open Play Store'),
          'none' => (Icons.watch_outlined, Tone.neutral, 'No Wear OS watch found', 'Pair a Wear OS watch with this phone in the Wear OS app first. No watch? Skip this.', null),
          _ => (Icons.watch_outlined, Tone.neutral, 'Looking for your watch', 'Checking the Wearable Data Layer.', null),
        };
        return _scaffold(
          top: [
            const SizedBox(height: 8),
            Center(
              child: AuraOrb(size: 150, tone: tone, icon: icon, live: st == 'connected')
                  .animate(key: ValueKey(st))
                  .scale(begin: const Offset(0.7, 0.7), curve: Springs.curve, duration: 700.ms),
            ),
            const SizedBox(height: 16),
            AnimatedSwitcher(
              duration: const Duration(milliseconds: 380),
              switchInCurve: Springs.smoothCurve,
              child: Column(
                key: ValueKey('w$st'),
                children: [
                  Text(
                    title,
                    key: const ValueKey('ob-watch-title'),
                    textAlign: TextAlign.center,
                    style: TextStyle(color: a.text, fontSize: 26, fontWeight: FontWeight.w800, letterSpacing: -0.7),
                  ),
                  const SizedBox(height: 8),
                  Text(body, textAlign: TextAlign.center, style: TextStyle(color: a.text2, fontSize: 15, height: 1.45)),
                  if (cta != null) ...[
                    const SizedBox(height: 18),
                    WideButton(cta, key: const ValueKey('ob-watch-install'), icon: Icons.open_in_new_rounded, onTap: sc.installWatchApp),
                  ],
                ],
              ),
            ),
          ],
          bottom: [
            const SizedBox(height: 20),
            WideButton(
              st == 'connected' ? 'Continue' : 'Skip for now',
              key: const ValueKey('ob-watch-next'),
              secondary: st != 'connected',
              onTap: () {
                Fx.tap();
                _go(4);
              },
            ),
          ],
        );
      },
    );
  }

  Widget _done() {
    final a = context.aura;
    return _scaffold(
      top: [
        const SizedBox(height: 24),
        Center(
          child: const AuraOrb(
            size: 200,
            tone: Tone.good,
            icon: Icons.done_all_rounded,
            live: true,
          ).animate().scale(begin: const Offset(0.7, 0.7), curve: Springs.curve, duration: 800.ms),
        ),
        const SizedBox(height: 16),
        Text(
          "You're all set",
          textAlign: TextAlign.center,
          style: TextStyle(color: a.text, fontSize: 32, fontWeight: FontWeight.w800, letterSpacing: -0.9),
        ),
        const SizedBox(height: 6),
        Text(
          'Aura now works by itself in the background.',
          textAlign: TextAlign.center,
          style: TextStyle(color: a.text2, fontSize: 15),
        ),
        const SizedBox(height: 24),
        AuraCard(
          child: Column(
            children: [
              for (final (i, t) in const [
                (Icons.widgets_rounded, 'Add the Aura widget: Lock, Unlock and Continue on PC from your home screen.'),
                (Icons.dashboard_customize_rounded, 'Add the Lock PC, Unlock PC and Continue on PC tiles to Quick Settings.'),
                (Icons.keyboard_rounded, 'On the PC, Ctrl+Alt+H sends what you are watching to this phone.'),
              ].indexed) ...[
                if (i > 0) const SizedBox(height: 14),
                Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Icon(t.$1, color: a.primary, size: 22),
                    const SizedBox(width: 12),
                    Expanded(
                      child: Text(t.$2, style: TextStyle(color: a.text2, fontSize: 14, height: 1.4)),
                    ),
                  ],
                ),
              ],
            ],
          ),
        ),
      ],
      bottom: [
        const SizedBox(height: 20),
        WideButton('Open Aura', key: const ValueKey('ob-done'), cue: Cue.success, onTap: widget.onDone),
      ],
    );
  }
}

class _Dots extends StatelessWidget {
  const _Dots({required this.step, required this.count});
  final int step;
  final int count;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (var i = 0; i < count; i++)
          AnimatedContainer(
            duration: const Duration(milliseconds: 380),
            curve: Springs.curve,
            margin: const EdgeInsets.symmetric(horizontal: 3),
            width: i == step ? 22 : 7,
            height: 7,
            decoration: BoxDecoration(color: i <= step ? a.primary : a.border.withValues(alpha: 0.4), borderRadius: BorderRadius.circular(4)),
          ),
      ],
    );
  }
}
