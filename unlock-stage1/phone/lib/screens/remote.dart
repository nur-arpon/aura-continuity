// Remote (item 15): music and volume on the PC with the now-playing card, the presentation remote,
// and Ring my laptop. One screen, three tabs; opened from Home. While it is open the PC pushes
// changes (it stops by itself two minutes after the screen closes), nothing polls in the background.
import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';

import '../core/dial.dart';
import '../core/fx.dart';
import '../core/model.dart';
import '../core/motion.dart';
import '../core/theme.dart';
import '../core/widgets.dart';
import 'inbox.dart' show PageHeader;
import 'laptop_apps.dart';
import 'settings.dart' show AuraSegmented;

/// Commands (mirror aura-link msg.rs).
abstract final class MediaCmd {
  static const playPause = 1, next = 2, prev = 3, volUp = 4, volDown = 5, mute = 6;
}

abstract final class PresentCmd {
  static const next = 1, prev = 2, black = 3, white = 4, start = 5, end = 6;
}

String fmtClock(int ms) {
  final s = (ms / 1000).floor().clamp(0, 359999);
  final m = s ~/ 60;
  final r = (s % 60).toString().padLeft(2, '0');
  return s >= 3600 ? '${s ~/ 3600}:${((s % 3600) ~/ 60).toString().padLeft(2, '0')}:$r' : '$m:$r';
}

class RemoteScreen extends StatefulWidget {
  const RemoteScreen({super.key, this.initialTab = 'music'});
  final String initialTab;
  @override
  State<RemoteScreen> createState() => _RemoteScreenState();
}

class _RemoteScreenState extends State<RemoteScreen> {
  late String tab = widget.initialTab;
  Timer? _poll;
  Timer? _keep;
  AuraModel? _m;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_m != null) return;
    final m = _m = AuraScope.of(context);
    m.remoteOpen();
    if (m.poll != null) {
      _poll = Timer.periodic(const Duration(milliseconds: 800), (_) => m.refreshRemote());
      _keep = Timer.periodic(const Duration(seconds: 60), (_) => m.remoteOpen());
    }
    m.setPresenterKeys(tab == 'slides');
  }

  @override
  void dispose() {
    _poll?.cancel();
    _keep?.cancel();
    _m?.remoteClose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final m = AuraScope.of(context);
    var i = 0;
    return Scaffold(
      body: AuraBackground(
        child: SafeArea(
          bottom: false,
          child: ListView(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 40),
            children: [
              const PageHeader(title: 'Remote', subtitle: 'Music, slides and finding things, from here'),
              const SizedBox(height: 14),
              _PowerRow(m: m),
              const SizedBox(height: 14),
              AuraSegmented<String>(
                key: const ValueKey('remote-tabs'),
                values: const ['music', 'slides', 'mouse', 'find'],
                labels: const ['Music', 'Slides', 'Mouse', 'Find'],
                value: tab,
                onChanged: (v) {
                  setState(() => tab = v);
                  m.setPresenterKeys(v == 'slides');
                },
              ),
              const SizedBox(height: 16),
              AnimatedSwitcher(
                duration: const Duration(milliseconds: 280),
                child: KeyedSubtree(
                  key: ValueKey(tab),
                  child: switch (tab) {
                    'slides' => _Slides(m: m, index: i),
                    'mouse' => _Mouse(m: m),
                    'find' => _Find(m: m),
                    _ => _Music(m: m),
                  },
                ),
              ),
              const SizedBox(height: 24),
              Text('Laptop apps', style: TextStyle(color: context.aura.text3, fontWeight: FontWeight.w600)),
              const SizedBox(height: 8),
              Wrap(spacing: 8, runSpacing: 8, children: [
                PillButton('Open or pin a laptop app', key: const ValueKey('laptop-apps'), icon: Icons.apps_rounded, onTap: () => showLaptopApps(context)),
              ]),
              const SizedBox(height: 16),
              Text('Add to home screen', style: TextStyle(color: context.aura.text3, fontWeight: FontWeight.w600)),
              const SizedBox(height: 8),
              Wrap(spacing: 8, runSpacing: 8, children: [
                PillButton('Wake PC', key: const ValueKey('pin-wake'), icon: Icons.power_settings_new_rounded, onTap: () => m.pinShortcut('wake')),
                PillButton('Lock PC', key: const ValueKey('pin-lock'), icon: Icons.lock_rounded, onTap: () => m.pinShortcut('lock')),
                PillButton('Show laptop', key: const ValueKey('pin-laptop'), icon: Icons.desktop_windows_rounded, onTap: () => m.pinShortcut('laptop')),
                PillButton('Touchpad', key: const ValueKey('pin-mouse'), icon: Icons.mouse_rounded, onTap: () => m.pinShortcut('mouse')),
              ]),
            ],
          ),
        ),
      ),
    );
  }
}

// ---------------------------------------------------------------------------------------- power

/// Wake / Sleep / Restart / Shut down. Restart and Shut down ask once (one tap to confirm, nothing to type).
class _PowerRow extends StatelessWidget {
  const _PowerRow({required this.m});
  final AuraModel m;

  Future<void> _confirm(BuildContext context, String action, String title, String body, String verb) async {
    Fx.tap();
    final a = context.aura;
    final ok = await showDialog<bool>(
      context: context,
      builder: (c) => AlertDialog(
        icon: Icon(Icons.power_settings_new_rounded, color: a.danger),
        title: Text(title),
        content: Text(body),
        actions: [
          TextButton(key: ValueKey('power-$action-cancel'), onPressed: () { Fx.tap(); Navigator.pop(c, false); }, child: const Text('Cancel')),
          TextButton(
            key: ValueKey('power-$action-confirm'),
            onPressed: () { Fx.tap(); Navigator.pop(c, true); },
            child: Text(verb, style: TextStyle(color: a.danger, fontWeight: FontWeight.w700)),
          ),
        ],
      ),
    );
    if (ok == true) await m.pcPower(action);
  }

  @override
  Widget build(BuildContext context) {
    final name = m.pcName == 'Your PC' ? 'your PC' : m.pcName;
    return Wrap(
      spacing: 8,
      runSpacing: 8,
      children: [
        PillButton('Wake', key: const ValueKey('pc-wake'), icon: Icons.power_settings_new_rounded, onTap: m.busy.contains('wake') ? null : m.wakePc),
        PillButton('Sleep', key: const ValueKey('pc-sleep'), icon: Icons.bedtime_rounded, onTap: () => m.pcPower('sleep')),
        PillButton('Restart', key: const ValueKey('pc-restart'), icon: Icons.restart_alt_rounded,
            onTap: () => _confirm(context, 'restart', 'Restart $name?', 'Anything unsaved on the PC is lost.', 'Restart')),
        PillButton('Shut down', key: const ValueKey('pc-shutdown'), icon: Icons.stop_circle_outlined,
            onTap: () => _confirm(context, 'shutdown', 'Shut down $name?', 'Anything unsaved on the PC is lost. Use Wake to turn it back on.', 'Shut down')),
      ],
    );
  }
}

// ---------------------------------------------------------------------------------------- music

class _Music extends StatelessWidget {
  const _Music({required this.m});
  final AuraModel m;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final r = m.remote;
    final title = (r['title'] as String?) ?? '';
    final artist = (r['artist'] as String?) ?? '';
    final playing = r['playing'] == true;
    final dur = (r['dur'] as int?) ?? 0;
    final pos = (r['pos'] as int?) ?? 0;
    final vol = (r['volume'] as int?) ?? -1;
    final muted = r['muted'] == true;
    final art = r['art'];
    final has = title.isNotEmpty;
    return Column(
      children: [
        AuraCard(
          child: Column(
            children: [
              Row(
                children: [
                  _Art(bytes: art is Uint8List ? art : null, playing: playing),
                  const SizedBox(width: 14),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          has ? title : 'Nothing playing',
                          key: const ValueKey('now-title'),
                          maxLines: 2,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(color: a.text, fontSize: 18, fontWeight: FontWeight.w700, letterSpacing: -0.3),
                        ),
                        const SizedBox(height: 3),
                        Text(
                          has ? (artist.isEmpty ? 'On your PC' : artist) : 'Start something on your PC',
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(color: a.text3, fontSize: 13.5),
                        ),
                      ],
                    ),
                  ),
                ],
              ),
              if (has && dur > 0) ...[
                const SizedBox(height: 16),
                ClipRRect(
                  borderRadius: BorderRadius.circular(99),
                  child: LinearProgressIndicator(
                    value: (pos / dur).clamp(0.0, 1.0),
                    minHeight: 5,
                    color: a.primary,
                    backgroundColor: a.primary.withValues(alpha: 0.16),
                  ),
                ),
                const SizedBox(height: 6),
                Row(
                  mainAxisAlignment: MainAxisAlignment.spaceBetween,
                  children: [
                    Text(fmtClock(pos), style: TextStyle(color: a.text3, fontSize: 12)),
                    Text(fmtClock(dur), style: TextStyle(color: a.text3, fontSize: 12)),
                  ],
                ),
              ],
              const SizedBox(height: 14),
              Row(
                mainAxisAlignment: MainAxisAlignment.spaceEvenly,
                children: [
                  _Round(key: const ValueKey('m-prev'), icon: Icons.skip_previous_rounded, label: 'Previous', onTap: () => m.mediaCmd(MediaCmd.prev)),
                  _Round(
                    key: const ValueKey('m-play'),
                    icon: playing ? Icons.pause_rounded : Icons.play_arrow_rounded,
                    label: playing ? 'Pause' : 'Play',
                    big: true,
                    onTap: () => m.mediaCmd(MediaCmd.playPause),
                  ),
                  _Round(key: const ValueKey('m-next'), icon: Icons.skip_next_rounded, label: 'Next', onTap: () => m.mediaCmd(MediaCmd.next)),
                ],
              ),
            ],
          ),
        ),
        const SizedBox(height: 12),
        AuraCard(
          radius: 22,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceEvenly,
            children: [
              _Round(key: const ValueKey('m-vdown'), icon: Icons.volume_down_rounded, label: 'Volume down', onTap: () => m.mediaCmd(MediaCmd.volDown)),
              Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  HapticDial(
                    value: vol,
                    size: 150,
                    onStep: (up) => m.volStep(up ? MediaCmd.volUp : MediaCmd.volDown),
                    center: Text(
                      muted ? 'Muted' : (vol >= 0 ? '$vol%' : 'Volume'),
                      key: const ValueKey('vol-text'),
                      style: TextStyle(color: a.text, fontSize: 20, fontWeight: FontWeight.w700),
                    ),
                  ),
                  PillButton(muted ? 'Unmute' : 'Mute', key: const ValueKey('m-mute'), cue: null, onTap: () => m.mediaCmd(MediaCmd.mute)),
                ],
              ),
              _Round(key: const ValueKey('m-vup'), icon: Icons.volume_up_rounded, label: 'Volume up', onTap: () => m.mediaCmd(MediaCmd.volUp)),
            ],
          ),
        ),
        const SizedBox(height: 14),
        Text(
          'Works with Spotify, YouTube in your browser, Media Player and most players on your PC.',
          textAlign: TextAlign.center,
          style: TextStyle(color: a.text3, fontSize: 12.5, height: 1.4),
        ),
      ],
    );
  }
}

class _Art extends StatelessWidget {
  const _Art({required this.bytes, required this.playing});
  final Uint8List? bytes;
  final bool playing;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return Container(
      width: 84,
      height: 84,
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(20),
        color: a.primary.withValues(alpha: 0.14),
        border: Border.all(color: a.divider),
      ),
      clipBehavior: Clip.antiAlias,
      child: bytes != null
          ? Image.memory(bytes!, key: const ValueKey('art'), fit: BoxFit.cover, gaplessPlayback: true, errorBuilder: (_, _, _) => Icon(Icons.music_note_rounded, color: a.primary, size: 36))
          : Icon(playing ? Icons.graphic_eq_rounded : Icons.music_note_rounded, color: a.primary, size: 36),
    );
  }
}

class _Round extends StatelessWidget {
  const _Round({super.key, required this.icon, required this.label, required this.onTap, this.big = false});
  final IconData icon;
  final String label;
  final VoidCallback onTap;
  final bool big;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final d = big ? 74.0 : 58.0;
    return Pressable(
      onTap: onTap,
      cue: Cue.tap,
      semanticLabel: label,
      scale: 0.9,
      child: Container(
        width: d,
        height: d,
        decoration: BoxDecoration(
          shape: BoxShape.circle,
          gradient: big ? LinearGradient(begin: Alignment.topLeft, end: Alignment.bottomRight, colors: [a.primaryBright, a.primary]) : null,
          color: big ? null : a.surface2,
          border: big ? null : Border.all(color: a.divider),
          boxShadow: big ? [BoxShadow(color: a.primary.withValues(alpha: 0.28), blurRadius: 20, offset: const Offset(0, 3))] : null,
        ),
        child: Icon(icon, size: big ? 38 : 28, color: big ? a.onPrimary : a.text),
      ),
    );
  }
}

// ---------------------------------------------------------------------------------------- slides

class _Slides extends StatelessWidget {
  const _Slides({required this.m, required this.index});
  final AuraModel m;
  final int index;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final keys = m.remote['presenterKeys'] == true;
    return Column(
      children: [
        Pressable(
          key: const ValueKey('p-next'),
          onTap: () => m.present(PresentCmd.next),
          cue: null,
          semanticLabel: 'Next slide',
          scale: 0.97,
          child: Container(
            height: 210,
            alignment: Alignment.center,
            decoration: BoxDecoration(
              borderRadius: BorderRadius.circular(30),
              gradient: LinearGradient(begin: Alignment.topLeft, end: Alignment.bottomRight, colors: [a.primaryBright, a.primary]),
              boxShadow: [BoxShadow(color: a.primary.withValues(alpha: 0.28), blurRadius: 26, offset: const Offset(0, 4))],
            ),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(Icons.arrow_forward_rounded, size: 64, color: a.onPrimary),
                const SizedBox(height: 6),
                Text('Next slide', style: TextStyle(color: a.onPrimary, fontSize: 22, fontWeight: FontWeight.w800, letterSpacing: -0.4)),
              ],
            ),
          ),
        ),
        const SizedBox(height: 12),
        Row(
          children: [
            Expanded(
              child: BigAction(
                key: const ValueKey('p-prev'),
                icon: Icons.arrow_back_rounded,
                label: 'Previous',
                caption: 'Back one slide',
                cue: null,
                onTap: () => m.present(PresentCmd.prev),
              ),
            ),
            const SizedBox(width: 12),
            Expanded(
              child: BigAction(
                key: const ValueKey('p-black'),
                icon: Icons.visibility_off_rounded,
                label: 'Blank',
                caption: 'Black screen on / off',
                cue: null,
                onTap: () => m.present(PresentCmd.black),
              ),
            ),
          ],
        ),
        const SizedBox(height: 12),
        Group(
          children: [
            Row2(
              rowKey: const ValueKey('p-start'),
              icon: Icons.slideshow_rounded,
              title: 'Start slideshow',
              subtitle: 'F5 on your PC',
              cue: null,
              onTap: () => m.present(PresentCmd.start),
              trailing: Icon(Icons.chevron_right_rounded, color: a.text3),
            ),
            Row2(
              rowKey: const ValueKey('p-end'),
              icon: Icons.stop_circle_outlined,
              title: 'End slideshow',
              subtitle: 'Esc on your PC',
              cue: null,
              onTap: () => m.present(PresentCmd.end),
              trailing: Icon(Icons.chevron_right_rounded, color: a.text3),
            ),
            Row2(
              rowKey: const ValueKey('p-white'),
              icon: Icons.light_mode_rounded,
              title: 'White screen',
              subtitle: 'W on your PC',
              cue: null,
              onTap: () => m.present(PresentCmd.white),
              trailing: Icon(Icons.chevron_right_rounded, color: a.text3),
            ),
          ],
        ),
        const SizedBox(height: 12),
        Group(
          children: [
            SwitchRow(
              switchKey: const ValueKey('p-volkeys'),
              icon: Icons.volume_down_rounded,
              title: 'Volume keys change slides',
              subtitle: 'Volume down = next, volume up = previous, while this tab is open',
              value: keys,
              onChanged: m.setPresenterKeys,
            ),
          ],
        ),
        const SizedBox(height: 12),
        Text(
          'Keys go to the window in front on your PC. Nothing is sent while the PC is locked.',
          textAlign: TextAlign.center,
          style: TextStyle(color: a.text3, fontSize: 12.5, height: 1.4),
        ),
      ],
    );
  }
}

// ---------------------------------------------------------------------------------------- find

// ---------------------------------------------------------------------------------------- mouse

/// Touchpad + keyboard for the PC (INPUT events). One finger drags the pointer (about 60 events a second at most),
/// tap = left click, long press = right click, two fingers = scroll.
class _Mouse extends StatefulWidget {
  const _Mouse({required this.m});
  final AuraModel m;
  @override
  State<_Mouse> createState() => _MouseState();
}

class _MouseState extends State<_Mouse> {
  final _text = TextEditingController();
  double _ax = 0, _ay = 0, _sy = 0;
  int _last = 0;
  Offset _prev = Offset.zero;

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  void _flush({bool force = false}) {
    final now = DateTime.now().millisecondsSinceEpoch;
    if (!force && now - _last < 16) return;
    final ix = _ax.truncate(), iy = _ay.truncate();
    if (ix == 0 && iy == 0) return;
    _ax -= ix;
    _ay -= iy;
    _last = now;
    widget.m.input({'t': 'move', 'dx': ix, 'dy': iy});
  }

  void _key(String k) {
    Fx.tap();
    widget.m.input({'t': 'key', 'k': k});
  }

  void _type() {
    final t = _text.text;
    if (t.isEmpty) return;
    widget.m.input({'t': 'text', 's': t});
    _text.clear();
  }

  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return Column(
      children: [
        Container(
          height: 280,
          width: double.infinity,
          decoration: BoxDecoration(color: a.surface1, borderRadius: BorderRadius.circular(22), border: Border.all(color: a.border)),
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () {
              Fx.tap();
              widget.m.input({'t': 'click', 'b': 'left'});
            },
            onLongPress: () {
              Fx.tap();
              widget.m.input({'t': 'click', 'b': 'right'});
            },
            onScaleStart: (d) => _prev = d.focalPoint,
            onScaleUpdate: (d) {
              final delta = d.focalPoint - _prev;
              _prev = d.focalPoint;
              if (d.pointerCount >= 2) {
                _sy += -delta.dy * 3;
                final n = _sy.truncate();
                if (n != 0) {
                  _sy -= n;
                  widget.m.input({'t': 'scroll', 'dy': n});
                }
              } else {
                _ax += delta.dx * 2;
                _ay += delta.dy * 2;
                _flush();
              }
            },
            onScaleEnd: (_) => _flush(force: true),
            child: Center(child: Text('Touchpad\ndrag = move · tap = click · hold = right click · two fingers = scroll', textAlign: TextAlign.center, style: TextStyle(color: a.text3))),
          ),
        ),
        const SizedBox(height: 12),
        Row(
          children: [
            Expanded(
              child: TextField(
                controller: _text,
                decoration: const InputDecoration(hintText: 'Type on the PC'),
                onSubmitted: (_) => _type(),
              ),
            ),
            IconButton(onPressed: () { Fx.tap(); _type(); }, icon: const Icon(Icons.send_rounded)),
            IconButton(onPressed: () { Fx.tap(); _key('backspace'); }, icon: const Icon(Icons.backspace_outlined)),
            IconButton(onPressed: () { Fx.tap(); _key('enter'); }, icon: const Icon(Icons.keyboard_return_rounded)),
          ],
        ),
      ],
    );
  }
}

class _Find extends StatelessWidget {
  const _Find({required this.m});
  final AuraModel m;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final ringing = m.remote['laptopRinging'] == true;
    final phoneRinging = m.remote['phoneRinging'] == true;
    return Column(
      children: [
        BigActionWide(
          key: const ValueKey('find-laptop'),
          icon: ringing ? Icons.notifications_off_rounded : Icons.notifications_active_rounded,
          label: ringing ? 'Stop ringing the laptop' : 'Ring my laptop',
          caption: ringing ? 'It is ringing now. Dismiss it on the laptop or here' : 'A loud chime and a full alert on the PC',
          primary: !ringing,
          busy: m.busy.contains('ring'),
          onTap: () => m.ringLaptop(!ringing),
        ),
        const SizedBox(height: 12),
        Group(
          children: [
            SwitchRow(
              switchKey: const ValueKey('allow-ring'),
              icon: Icons.phone_android_rounded,
              title: 'Let my PC ring this phone',
              subtitle: 'From the Aura tray menu on the PC. Rings at full volume, even on silent, until you touch the screen',
              value: m.remote['allowRing'] != false,
              onChanged: m.setAllowRing,
            ),
            if (phoneRinging)
              Row2(
                rowKey: const ValueKey('stop-phone-ring'),
                icon: Icons.notifications_off_rounded,
                tone: Tone.warn,
                title: 'This phone is ringing',
                subtitle: 'Tap to stop it',
                onTap: m.stopRinging,
              ),
          ],
        ),
        const SizedBox(height: 14),
        Text(
          'To find your phone, use the Aura icon on the PC: tray menu > Ring my phone. Your watch can find the phone with Samsung\'s own Find my phone.',
          textAlign: TextAlign.center,
          style: TextStyle(color: a.text3, fontSize: 12.5, height: 1.4),
        ),
      ],
    );
  }
}

/// A full-width version of the Home action tile.
class BigActionWide extends StatelessWidget {
  const BigActionWide({super.key, required this.icon, required this.label, required this.caption, required this.onTap, this.primary = false, this.busy = false});
  final IconData icon;
  final String label;
  final String caption;
  final VoidCallback onTap;
  final bool primary;
  final bool busy;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final fg = primary ? a.onPrimary : a.text;
    return Pressable(
      onTap: busy ? null : onTap,
      cue: null,
      semanticLabel: label,
      child: Container(
        padding: const EdgeInsets.fromLTRB(18, 18, 16, 18),
        decoration: BoxDecoration(
          borderRadius: BorderRadius.circular(26),
          gradient: primary ? LinearGradient(begin: Alignment.topLeft, end: Alignment.bottomRight, colors: [a.primaryBright, a.primary]) : null,
          color: primary ? null : a.surface2,
          border: primary ? null : Border.all(color: a.divider),
          boxShadow: primary ? [BoxShadow(color: a.primary.withValues(alpha: 0.26), blurRadius: 22, offset: const Offset(0, 3))] : null,
        ),
        child: Row(
          children: [
            busy ? SizedBox.square(dimension: 34, child: CircularProgressIndicator(strokeWidth: 2.6, color: fg)) : Icon(icon, size: 34, color: primary ? fg : a.primary),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(label, style: TextStyle(color: fg, fontSize: 19, fontWeight: FontWeight.w700, letterSpacing: -0.3)),
                  const SizedBox(height: 2),
                  Text(caption, style: TextStyle(color: primary ? a.onPrimary.withValues(alpha: 0.8) : a.text3, fontSize: 12.8, height: 1.3)),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
