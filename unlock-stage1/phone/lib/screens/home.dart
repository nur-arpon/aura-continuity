// Home: the PC at a glance (nearby / away / locked), the two big buttons, Continue from laptop,
// Continue on PC, and the latest Handoff items. Everything one tap away, nothing hidden.
import 'package:flutter/material.dart';

import '../core/fx.dart';
import '../core/model.dart';
import '../core/motion.dart';
import '../core/status.dart';
import '../core/theme.dart';
import '../core/widgets.dart';
import 'inbox.dart';
import 'update_row.dart';
import 'remote.dart';
import 'settings.dart';

part 'home_expressive.dart';

class HomeScreen extends StatelessWidget {
  const HomeScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final m = AuraScope.of(context);
    final a = context.aura;
    final st = m.status;
    final pending = (m.s['pending'] as String?) ?? '';
    final unseen = (m.s['inboxUnseen'] as int?) ?? 0;
    final missing = ((m.s['missingPerms'] as List?) ?? const []).isNotEmpty;
    final companion = m.s['companion'] == true || m.s['companionSupported'] != true;
    final away = (m.a['code'] as String?) ?? '';
    if (a.expressive) return _ExpressiveHome(m: m, unseen: unseen, pending: pending, missing: missing, companion: companion, away: away);
    var i = 0;
    return Scaffold(
      body: AuraBackground(
        child: SafeArea(
          bottom: false,
          child: RefreshIndicator(
            color: a.primary,
            backgroundColor: a.surface1,
            onRefresh: () async {
              Fx.tick();
              await m.askPc();
            },
            child: ListView(
              key: const ValueKey('home-list'),
              padding: const EdgeInsets.fromLTRB(16, 4, 16, 40),
              children: [
                _TopBar(unseen: unseen),
                const UpdateRow(rowKey: ValueKey('update-home')),
                if (pending.isNotEmpty)
                  Entrance(
                    index: i++,
                    child: _ApproveCard(via: pending),
                  ),
                Entrance(
                  index: i++,
                  child: _Hero(st: st, name: m.pcName),
                ),
                const SizedBox(height: 14),
                if (!m.extraPhone) Entrance(
                  index: i++,
                  child: Row(
                    children: [
                      Expanded(
                        child: BigAction(
                          key: const ValueKey('lock'),
                          icon: Icons.lock_rounded,
                          label: 'Lock PC',
                          caption: st.kind == PcKind.locked ? 'Already locked' : 'One tap, from anywhere',
                          primary: st.kind != PcKind.locked,
                          busy: m.busy.contains('lock'),
                          cue: null, // the model plays the lock "thunk" itself
                          onTap: m.lockPc,
                        ),
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: BigAction(
                          key: const ValueKey('unlock'),
                          icon: Icons.fingerprint_rounded,
                          label: 'Unlock PC',
                          caption: m.s['unlockBusy'] == true ? 'Waiting for your PC…' : 'With your fingerprint',
                          primary: st.kind == PcKind.locked,
                          busy: m.s['unlockBusy'] == true,
                          onTap: m.unlockPc,
                        ),
                      ),
                    ],
                  ),
                ),
                const SectionLabel('Remote'),
                Entrance(
                  index: i++,
                  child: Row(
                    children: [
                      Expanded(
                        child: _QuickAction(
                          key: const ValueKey('remote-music'),
                          icon: Icons.music_note_rounded,
                          title: 'Music & slides',
                          sub: 'Play, volume, next slide',
                          onTap: () => Navigator.of(context).push(auraRoute(const RemoteScreen())),
                        ),
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: _QuickAction(
                          key: const ValueKey('remote-ring'),
                          icon: Icons.notifications_active_rounded,
                          title: 'Ring my laptop',
                          sub: 'Find it by sound',
                          busy: m.busy.contains('ring'),
                          onTap: () => m.ringLaptop(true),
                        ),
                      ),
                    ],
                  ),
                ),
                if (missing || !companion)
                  Entrance(
                    index: i++,
                    child: _FinishSetup(missing: missing),
                  ),
                const SectionLabel('Continue'),
                Entrance(
                  index: i++,
                  child: _FromLaptop(m: m),
                ),
                if (((m.a['recent'] as List?) ?? const []).isNotEmpty) _MoreFromPc(m: m),
                const SizedBox(height: 12),
                Entrance(
                  index: i++,
                  child: Row(
                    children: [
                      Expanded(
                        child: _QuickAction(
                          key: const ValueKey('topc'),
                          icon: Icons.cast_rounded,
                          title: 'Continue on PC',
                          sub: 'Video, page or link',
                          busy: m.busy.contains('topc'),
                          onTap: m.continueOnPc,
                        ),
                      ),
                      const SizedBox(width: 12),
                      Expanded(
                        child: _QuickAction(
                          key: const ValueKey('clip'),
                          icon: Icons.content_paste_go_rounded,
                          title: 'Clipboard to PC',
                          sub: 'Paste it on the PC',
                          busy: m.busy.contains('clip'),
                          onTap: m.clipboardToPc,
                        ),
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 12),
                Entrance(
                  index: i++,
                  child: _QuickAction(
                    key: const ValueKey('open-laptop'),
                    icon: Icons.desktop_windows_rounded,
                    title: 'Open laptop',
                    sub: 'See and control its screen (Moonlight)',
                    busy: m.busy.contains('laptop'),
                    onTap: m.openLaptop,
                  ),
                ),
                if (away.isNotEmpty && away != 'ready' && away != 'unpaired')
                  Entrance(
                    index: i++,
                    child: _AwayCard(m: m),
                  ),
                SectionLabel('Handoff', trailing: m.inbox.isEmpty ? null : _SeeAll(unseen: unseen)),
                Entrance(
                  index: i++,
                  child: InboxPreview(items: m.inbox.take(3).toList()),
                ),
                const SizedBox(height: 28),
                Center(
                  child: Text('Aura · Continuity for your PC', style: TextStyle(color: a.text3.withValues(alpha: 0.8), fontSize: 12)),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

class _TopBar extends StatelessWidget {
  const _TopBar({required this.unseen});
  final int unseen;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return Padding(
      padding: const EdgeInsets.fromLTRB(4, 10, 0, 6),
      child: Row(
        children: [
          const AuraOrb(size: 30, live: false),
          const SizedBox(width: 8),
          Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                'Aura',
                style: TextStyle(color: a.text, fontSize: 24, fontWeight: FontWeight.w800, letterSpacing: -0.6, height: 1.1),
              ),
              Text(
                'Continuity for your PC',
                style: TextStyle(color: a.text3, fontSize: 12.5, fontWeight: FontWeight.w500),
              ),
            ],
          ),
          const Spacer(),
          _RoundIcon(
            key: const ValueKey('open-inbox'),
            icon: Icons.inbox_rounded,
            badge: unseen,
            label: 'Handoff inbox',
            onTap: () => Navigator.of(context).push(auraRoute(const InboxScreen())),
          ),
          const SizedBox(width: 8),
          _RoundIcon(
            key: const ValueKey('open-settings'),
            icon: Icons.tune_rounded,
            label: 'Settings',
            onTap: () => Navigator.of(context).push(auraRoute(const SettingsScreen())),
          ),
        ],
      ),
    );
  }
}

class _RoundIcon extends StatelessWidget {
  const _RoundIcon({super.key, required this.icon, required this.onTap, required this.label, this.badge = 0});
  final IconData icon;
  final VoidCallback onTap;
  final String label;
  final int badge;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return Pressable(
      onTap: onTap,
      semanticLabel: label,
      scale: 0.9,
      child: SizedBox.square(
        dimension: 46,
        child: Stack(
          clipBehavior: Clip.none,
          children: [
            Container(
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: a.surface1.withValues(alpha: a.dark ? 0.7 : 0.9),
                border: Border.all(color: a.divider),
              ),
              child: Center(child: Icon(icon, color: a.text2, size: 22)),
            ),
            if (badge > 0)
              Positioned(
                right: -2,
                top: -2,
                child: Container(
                  constraints: const BoxConstraints(minWidth: 20),
                  height: 20,
                  padding: const EdgeInsets.symmetric(horizontal: 5),
                  decoration: BoxDecoration(
                    color: a.primary,
                    borderRadius: BorderRadius.circular(10),
                    border: Border.all(color: a.bg, width: 2),
                  ),
                  child: Center(
                    child: Text(
                      badge > 9 ? '9+' : '$badge',
                      style: TextStyle(color: a.onPrimary, fontSize: 10.5, fontWeight: FontWeight.w800),
                    ),
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _Hero extends StatelessWidget {
  const _Hero({required this.st, required this.name});
  final PcStatus st;
  final String name;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final live = st.kind == PcKind.nearby || st.kind == PcKind.away || st.kind == PcKind.unlocking;
    final m = AuraScope.read(context);
    return Pressable(
      key: const ValueKey('hero'),
      onTap: m.askPc,
      cue: null,
      scale: 0.985,
      semanticLabel: '$name ${st.title}. ${st.subtitle}. Tap to check again.',
      child: AuraCard(
        padding: const EdgeInsets.fromLTRB(20, 20, 20, 22),
        radius: 30,
        child: Column(
          children: [
            Row(
              children: [
                Icon(Icons.laptop_windows_rounded, size: 16, color: a.text3),
                const SizedBox(width: 6),
                Text(
                  name.toUpperCase(),
                  style: TextStyle(color: a.text3, fontSize: 12.5, fontWeight: FontWeight.w700, letterSpacing: 1.3),
                ),
                const Spacer(),
                AnimatedSwitcher(
                  duration: const Duration(milliseconds: 300),
                  child: m.busy.contains('ask')
                      ? SizedBox.square(
                          key: const ValueKey('spin'),
                          dimension: 16,
                          child: CircularProgressIndicator(strokeWidth: 2, color: a.text3),
                        )
                      : Icon(Icons.refresh_rounded, key: const ValueKey('r'), size: 18, color: a.text3),
                ),
              ],
            ),
            const SizedBox(height: 4),
            AuraOrb(size: 168, tone: st.tone, icon: st.icon, live: live),
            const SizedBox(height: 4),
            AnimatedSwitcher(
              duration: const Duration(milliseconds: 360),
              switchInCurve: Springs.curve,
              transitionBuilder: (c, an) => FadeTransition(
                opacity: an,
                child: SlideTransition(
                  position: Tween(begin: const Offset(0, 0.25), end: Offset.zero).animate(an),
                  child: c,
                ),
              ),
              child: Column(
                key: ValueKey(st.title + st.subtitle),
                children: [
                  Text(
                    st.title,
                    style: TextStyle(color: a.text, fontSize: 34, fontWeight: FontWeight.w800, letterSpacing: -1, height: 1.1),
                  ),
                  const SizedBox(height: 8),
                  Pill(st.subtitle, tone: st.tone),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _ApproveCard extends StatelessWidget {
  const _ApproveCard({required this.via});
  final String via;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final m = AuraScope.read(context);
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: AuraCard(
        color: Color.alphaBlend(a.primary.withValues(alpha: a.dark ? 0.16 : 0.08), a.surface1),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                const IconTile(Icons.lock_open_rounded),
                const SizedBox(width: 12),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        'Your PC asks to unlock',
                        style: TextStyle(color: a.text, fontSize: 17, fontWeight: FontWeight.w700),
                      ),
                      Text('Request over $via · only your paired PC can ask', style: TextStyle(color: a.text3, fontSize: 12.8)),
                    ],
                  ),
                ),
              ],
            ),
            const SizedBox(height: 14),
            Row(
              children: [
                Expanded(
                  child: WideButton('Approve', key: const ValueKey('approve'), icon: Icons.check_rounded, cue: null, onTap: m.approve),
                ),
                const SizedBox(width: 10),
                Expanded(
                  child: WideButton('Not now', key: const ValueKey('deny'), secondary: true, cue: null, onTap: m.deny),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _FinishSetup extends StatelessWidget {
  const _FinishSetup({required this.missing});
  final bool missing;
  @override
  Widget build(BuildContext context) {
    final m = AuraScope.read(context);
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Group(
        children: [
          Row2(
            rowKey: const ValueKey('finish-setup'),
            icon: missing ? Icons.bluetooth_searching_rounded : Icons.bolt_rounded,
            tone: Tone.warn,
            title: missing ? 'Let Aura find your PC' : 'Keep Aura always ready',
            subtitle: missing ? 'Allow Nearby devices and notifications.' : 'Link your PC once so Aura starts by itself after a restart.',
            trailing: PillButton(missing ? 'Allow' : 'Link', cue: null, onTap: missing ? m.requestPerms : m.linkCompanion),
            onTap: missing ? m.requestPerms : m.linkCompanion,
          ),
        ],
      ),
    );
  }
}

class _FromLaptop extends StatelessWidget {
  const _FromLaptop({required this.m});
  final AuraModel m;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final title = (m.a['lastTitle'] as String?) ?? '';
    final sub = (m.a['lastSub'] as String?) ?? '';
    final yt = m.a['lastApp'] == 'youtube';
    final has = title.isNotEmpty;
    final busy = m.busy.contains('continue');
    final cs = Theme.of(context).colorScheme;
    return Pressable(
      key: const ValueKey('from-laptop'),
      onTap: has ? m.continueFromLaptop : m.askPc,
      cue: Cue.tap,
      scale: 0.98,
      semanticLabel: has ? 'Continue $title from your PC' : 'Ask your PC what it was doing',
      child: AuraCard(
        padding: const EdgeInsets.all(14),
        color: a.expressive ? a.surface3 : null,
        radius: a.expressive ? 30 : 26,
        child: Row(
          children: [
            Container(
              width: 64,
              height: 64,
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(a.expressive ? 26 : 18),
                color: a.expressive && !yt ? cs.primaryContainer : null,
                gradient: a.expressive && !yt
                    ? null
                    : LinearGradient(
                        begin: Alignment.topLeft,
                        end: Alignment.bottomRight,
                        colors: yt ? [const Color(0xFFFF5A5A), const Color(0xFFC81E1E)] : [a.primaryBright, a.primary],
                      ),
              ),
              child: Icon(
                yt ? Icons.smart_display_rounded : (has ? Icons.public_rounded : Icons.laptop_chromebook_rounded),
                color: a.expressive && !yt ? cs.onPrimaryContainer : Colors.white,
                size: 30,
              ),
            ),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'FROM YOUR PC',
                    style: TextStyle(color: a.text3, fontSize: 11, fontWeight: FontWeight.w700, letterSpacing: 1.1),
                  ),
                  const SizedBox(height: 3),
                  Text(
                    has ? title : 'Nothing to continue yet',
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(color: a.text, fontSize: 16, fontWeight: FontWeight.w700, height: 1.25),
                  ),
                  const SizedBox(height: 3),
                  Text(
                    has ? '$sub${m.a['pcLocked'] == true ? ' · PC locked' : ''}' : 'Pull down or tap to ask your PC',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(color: a.text3, fontSize: 13),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 10),
            Container(
              width: 48,
              height: 48,
              decoration: BoxDecoration(shape: BoxShape.circle, color: has ? a.primary : a.primary.withValues(alpha: 0.14)),
              child: Center(
                child: busy
                    ? SizedBox.square(dimension: 20, child: CircularProgressIndicator(strokeWidth: 2.4, color: has ? a.onPrimary : a.primary))
                    : Icon(has ? Icons.play_arrow_rounded : Icons.refresh_rounded, color: has ? a.onPrimary : a.primary, size: 28),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _QuickAction extends StatelessWidget {
  const _QuickAction({super.key, required this.icon, required this.title, required this.sub, required this.onTap, this.busy = false});
  final IconData icon;
  final String title;
  final String sub;
  final VoidCallback onTap;
  final bool busy;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return Pressable(
      onTap: busy ? null : onTap,
      cue: null, // these actions play their own "send" cue
      semanticLabel: title,
      child: AuraCard(
        padding: const EdgeInsets.fromLTRB(14, 14, 12, 14),
        radius: 22,
        child: Row(
          children: [
            busy
                ? SizedBox.square(
                    dimension: 36,
                    child: Padding(
                      padding: const EdgeInsets.all(8),
                      child: CircularProgressIndicator(strokeWidth: 2.2, color: a.primary),
                    ),
                  )
                : IconTile(icon),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    title,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(color: a.text, fontSize: 14.5, fontWeight: FontWeight.w700),
                  ),
                  Text(
                    sub,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(color: a.text3, fontSize: 12),
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _AwayCard extends StatelessWidget {
  const _AwayCard({required this.m});
  final AuraModel m;
  @override
  Widget build(BuildContext context) {
    final code = (m.a['code'] as String?) ?? '';
    final action = (m.a['action'] as String?) ?? 'none';
    return Padding(
      padding: const EdgeInsets.only(top: 12),
      child: Group(
        children: [
          Row2(
            rowKey: const ValueKey('away-card'),
            icon: Icons.public_off_rounded,
            tone: Tone.warn,
            title: (m.a['title'] as String?) ?? 'Away mode',
            subtitle: (m.a['text'] as String?) ?? '',
            onTap: action == 'none' ? null : () => m.bridge.call('tailscale'),
            trailing: action == 'none'
                ? null
                : PillButton(action == 'install' ? 'Install' : (code == 'off' ? 'Turn on' : 'Open'), cue: Cue.tap, onTap: () => m.bridge.call('tailscale')),
          ),
        ],
      ),
    );
  }
}

class _SeeAll extends StatelessWidget {
  const _SeeAll({required this.unseen});
  final int unseen;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return Pressable(
      key: const ValueKey('see-all'),
      onTap: () => Navigator.of(context).push(auraRoute(const InboxScreen())),
      semanticLabel: 'See all Handoff items',
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
        child: Text(
          unseen > 0 ? 'See all · $unseen new' : 'See all',
          style: TextStyle(color: a.primary, fontSize: 13.5, fontWeight: FontWeight.w600),
        ),
      ),
    );
  }
}

/// "More from your PC": everything else recent (other windows, open tabs, the last hour), expandable.
class _MoreFromPc extends StatefulWidget {
  const _MoreFromPc({required this.m});
  final AuraModel m;
  @override
  State<_MoreFromPc> createState() => _MoreFromPcState();
}

class _MoreFromPcState extends State<_MoreFromPc> {
  bool _open = false;

  /// Long-press: open this one in the app or in the browser, whatever the setting says.
  Future<void> _chooseOpen(int i) async {
    final mode = await showModalBottomSheet<String>(
      context: context,
      builder: (ctx) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(key: const ValueKey('open-app'), leading: const Icon(Icons.apps_rounded), title: const Text('Open in the app'), onTap: () { Fx.tap(); Navigator.pop(ctx, 'app'); }),
            ListTile(key: const ValueKey('open-browser'), leading: const Icon(Icons.public_rounded), title: const Text('Open in the browser'), onTap: () { Fx.tap(); Navigator.pop(ctx, 'browser'); }),
          ],
        ),
      ),
    );
    if (mode != null) widget.m.openRecent(i, mode: mode);
  }

  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final items = ((widget.m.a['recent'] as List?) ?? const []).map((e) => Map<String, dynamic>.from(e as Map)).toList();
    return Padding(
      padding: const EdgeInsets.only(top: 10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Pressable(
            key: const ValueKey('more-from-pc'),
            onTap: () => setState(() => _open = !_open),
            cue: Cue.tap,
            scale: 0.98,
            semanticLabel: 'More from your PC, ${items.length} items',
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 8),
              child: Row(
                children: [
                  Text('More from your PC (${items.length})', style: TextStyle(color: a.text2, fontSize: 14, fontWeight: FontWeight.w700)),
                  const Spacer(),
                  Icon(_open ? Icons.expand_less_rounded : Icons.expand_more_rounded, color: a.text3),
                ],
              ),
            ),
          ),
          if (_open)
            Group(
              children: [
                for (var i = 0; i < items.length; i++)
                  Row2(
                    rowKey: ValueKey('recent-$i'),
                    icon: items[i]['app'] == 'youtube' ? Icons.smart_display_rounded : Icons.public_rounded,
                    tone: items[i]['app'] == 'youtube' ? Tone.warn : Tone.info,
                    title: '${items[i]['title']}',
                    subtitle: [items[i]['sub'], items[i]['ago']].where((x) => x != null && '$x'.isNotEmpty).join(' · '),
                    trailing: Icon(Icons.play_arrow_rounded, color: a.primary),
                    onTap: () => widget.m.openRecent(i),
                    onLongPress: () => _chooseOpen(i),
                  ),
              ],
            ),
        ],
      ),
    );
  }
}
