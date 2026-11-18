// Expressive home (Material 3): one hero card for the PC, one big action, a quick-actions grid,
// the devices, then what continues from the PC. Classic keeps the layout in home.dart.
part of 'home.dart';

class _ExpressiveHome extends StatelessWidget {
  const _ExpressiveHome({required this.m, required this.unseen, required this.pending, required this.missing, required this.companion, required this.away});
  final AuraModel m;
  final int unseen;
  final String pending;
  final bool missing;
  final bool companion;
  final String away;

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    var i = 0;
    return Scaffold(
      body: SafeArea(
        bottom: false,
        child: RefreshIndicator(
          color: cs.primary,
          backgroundColor: cs.surfaceContainerHigh,
          onRefresh: () async {
            Fx.tick();
            await m.askPc();
          },
          child: ListView(
            key: const ValueKey('home-list'),
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 48),
            children: [
              _ExTopBar(unseen: unseen),
              const UpdateRow(rowKey: ValueKey('update-home')),
              if (pending.isNotEmpty) Entrance(index: i++, child: _ApproveCard(via: pending)),
              Entrance(index: i++, child: _ExHero(m: m)),
              const SectionLabel('Quick actions'),
              Entrance(index: i++, child: _ExGrid(m: m)),
              if (missing || !companion) Entrance(index: i++, child: _FinishSetup(missing: missing)),
              const SectionLabel('Continue'),
              Entrance(index: i++, child: _FromLaptop(m: m)),
              if (((m.a['recent'] as List?) ?? const []).isNotEmpty) _MoreFromPc(m: m),
              const SizedBox(height: 12),
              Entrance(
                index: i++,
                child: Row(
                  children: [
                    Expanded(
                      child: _ExWide(key: const ValueKey('topc'), icon: Icons.cast_rounded, label: 'Continue on PC', busy: m.busy.contains('topc'), onTap: m.continueOnPc),
                    ),
                    const SizedBox(width: 10),
                    Expanded(
                      child: _ExWide(key: const ValueKey('clip'), icon: Icons.content_paste_go_rounded, label: 'Clipboard to PC', busy: m.busy.contains('clip'), onTap: m.clipboardToPc),
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 10),
              Entrance(
                index: i++,
                child: _ExWide(
                  key: const ValueKey('remote-music'),
                  icon: Icons.music_note_rounded,
                  label: 'Music & slides',
                  cue: Cue.tap,
                  onTap: () => Navigator.of(context).push(auraRoute(const RemoteScreen())),
                ),
              ),
              const SectionLabel('Devices'),
              Entrance(index: i++, child: _ExDevices(m: m)),
              if (away.isNotEmpty && away != 'ready' && away != 'unpaired') Entrance(index: i++, child: _AwayCard(m: m)),
              SectionLabel('Handoff', trailing: m.inbox.isEmpty ? null : _SeeAll(unseen: unseen)),
              Entrance(index: i++, child: InboxPreview(items: m.inbox.take(3).toList())),
              const SizedBox(height: 28),
              Center(child: Text('Aura · Continuity for your PC', style: Theme.of(context).textTheme.bodySmall)),
            ],
          ),
        ),
      ),
    );
  }
}

class _ExTopBar extends StatelessWidget {
  const _ExTopBar({required this.unseen});
  final int unseen;
  @override
  Widget build(BuildContext context) {
    final tt = Theme.of(context).textTheme;
    return Padding(
      padding: const EdgeInsets.fromLTRB(6, 10, 0, 14),
      child: Row(
        children: [
          const AuraOrb(size: 40, live: false),
          const SizedBox(width: 10),
          Text('Aura', style: tt.headlineMedium?.copyWith(color: Theme.of(context).colorScheme.onSurface)),
          const Spacer(),
          _ExIconButton(
            key: const ValueKey('open-inbox'),
            icon: Icons.inbox_rounded,
            badge: unseen,
            label: 'Handoff inbox',
            onTap: () => Navigator.of(context).push(auraRoute(const InboxScreen())),
          ),
          const SizedBox(width: 8),
          _ExIconButton(
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

class _ExIconButton extends StatelessWidget {
  const _ExIconButton({super.key, required this.icon, required this.onTap, required this.label, this.badge = 0});
  final IconData icon;
  final VoidCallback onTap;
  final String label;
  final int badge;
  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    return Pressable(
      onTap: onTap,
      semanticLabel: label,
      scale: 0.88,
      child: SizedBox.square(
        dimension: 52,
        child: Stack(
          clipBehavior: Clip.none,
          children: [
            Container(
              decoration: ShapeDecoration(color: cs.secondaryContainer, shape: const RoundedSuperellipseBorder(borderRadius: BorderRadius.all(Radius.circular(22)))),
              child: Center(child: Icon(icon, color: cs.onSecondaryContainer, size: 24)),
            ),
            if (badge > 0)
              Positioned(
                right: -3,
                top: -3,
                child: Container(
                  constraints: const BoxConstraints(minWidth: 22),
                  height: 22,
                  padding: const EdgeInsets.symmetric(horizontal: 6),
                  decoration: BoxDecoration(color: cs.primary, borderRadius: BorderRadius.circular(11), border: Border.all(color: cs.surface, width: 2)),
                  child: Center(child: Text(badge > 9 ? '9+' : '$badge', style: TextStyle(color: cs.onPrimary, fontSize: 11, fontWeight: FontWeight.w800))),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// The PC at a glance: a big tonal card whose colour and corner shape follow its state, the link
/// it is using, and the one thing you most likely want to do (Unlock when locked, Lock otherwise).
class _ExHero extends StatelessWidget {
  const _ExHero({required this.m});
  final AuraModel m;

  static (Color, Color) _colors(ColorScheme cs, PcKind k) => switch (k) {
    PcKind.nearby => (cs.tertiaryContainer, cs.onTertiaryContainer),
    PcKind.away || PcKind.reachable => (cs.secondaryContainer, cs.onSecondaryContainer),
    PcKind.outOfReach || PcKind.unpaired => (cs.errorContainer, cs.onErrorContainer),
    _ => (cs.primaryContainer, cs.onPrimaryContainer),
  };

  static BorderRadius _shape(PcKind k) => switch (k) {
    PcKind.locked => const BorderRadius.only(topLeft: Radius.circular(56), topRight: Radius.circular(28), bottomLeft: Radius.circular(28), bottomRight: Radius.circular(56)),
    PcKind.nearby => const BorderRadius.only(topLeft: Radius.circular(28), topRight: Radius.circular(56), bottomLeft: Radius.circular(56), bottomRight: Radius.circular(28)),
    _ => BorderRadius.circular(44),
  };

  static (IconData, String) _via(Via v) => switch (v) {
    Via.bluetooth => (Icons.bluetooth_rounded, 'Bluetooth'),
    Via.lan => (Icons.wifi_rounded, 'Wi-Fi'),
    Via.tailnet => (Icons.vpn_lock_rounded, 'Tailscale'),
    Via.none => (Icons.link_off_rounded, 'No link'),
  };

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    final tt = Theme.of(context).textTheme;
    final st = m.status;
    final (bg, fg) = _colors(cs, st.kind);
    final via = viaOf((m.s['linkVia'] as String?) ?? '');
    final (viaIcon, viaLabel) = _via(via);
    final live = st.kind == PcKind.nearby || st.kind == PcKind.away || st.kind == PcKind.unlocking;
    final locked = st.kind == PcKind.locked || st.kind == PcKind.unlocking;
    final unlocking = m.s['unlockBusy'] == true;

    Widget action(String keyName, IconData icon, String label, {required bool primary, required bool busy, required VoidCallback onTap, Cue? cue = Cue.tap}) {
      void press() {
        if (cue != null) Fx.play(cue);
        onTap();
      }

      final spin = SizedBox.square(dimension: 20, child: CircularProgressIndicator(strokeWidth: 2.4, color: primary ? bg : fg));
      final text = Text(label, maxLines: 1, overflow: TextOverflow.ellipsis);
      return primary
          ? FilledButton.icon(
              key: ValueKey(keyName),
              style: FilledButton.styleFrom(backgroundColor: fg, foregroundColor: bg, minimumSize: const Size.fromHeight(64)),
              onPressed: busy ? null : press,
              icon: busy ? spin : Icon(icon, size: 24),
              label: text,
            )
          : FilledButton.tonalIcon(
              key: ValueKey(keyName),
              style: FilledButton.styleFrom(backgroundColor: fg.withValues(alpha: 0.14), foregroundColor: fg, minimumSize: const Size.fromHeight(64)),
              onPressed: busy ? null : press,
              icon: busy ? spin : Icon(icon, size: 22),
              label: text,
            );
    }

    final lockBtn = action('lock', Icons.lock_rounded, locked ? 'Lock' : 'Lock PC', primary: !locked, busy: m.busy.contains('lock'), cue: null, onTap: m.lockPc);
    final unlockBtn = action('unlock', Icons.fingerprint_rounded, unlocking ? 'Waiting…' : (locked ? 'Unlock PC' : 'Unlock'), primary: locked, busy: unlocking, onTap: m.unlockPc);

    return Pressable(
      key: const ValueKey('hero'),
      onTap: m.askPc,
      cue: null,
      scale: 0.99,
      semanticLabel: '${m.pcName} ${st.title}. ${st.subtitle}. Tap to check again.',
      child: AnimatedContainer(
        duration: const Duration(milliseconds: 700),
        curve: Springs.curve,
        padding: const EdgeInsets.fromLTRB(22, 20, 22, 22),
        decoration: BoxDecoration(color: bg, borderRadius: _shape(st.kind)),
        child: DefaultTextStyle.merge(
          style: TextStyle(color: fg),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Icon(Icons.laptop_windows_rounded, size: 20, color: fg.withValues(alpha: 0.8)),
                  const SizedBox(width: 8),
                  Expanded(child: Text(m.pcName, maxLines: 1, overflow: TextOverflow.ellipsis, style: tt.titleMedium?.copyWith(color: fg.withValues(alpha: 0.85)))),
                  AnimatedSwitcher(
                    duration: const Duration(milliseconds: 300),
                    child: m.busy.contains('ask')
                        ? SizedBox.square(key: const ValueKey('spin'), dimension: 18, child: CircularProgressIndicator(strokeWidth: 2.2, color: fg))
                        : Icon(Icons.refresh_rounded, key: const ValueKey('r'), size: 20, color: fg.withValues(alpha: 0.7)),
                  ),
                ],
              ),
              const SizedBox(height: 18),
              Row(
                children: [
                  _StateBadge(icon: st.icon, color: fg, on: bg, live: live),
                  const SizedBox(width: 16),
                  Expanded(
                    child: AnimatedSwitcher(
                      duration: const Duration(milliseconds: 420),
                      switchInCurve: Springs.curve,
                      transitionBuilder: (c, an) => FadeTransition(opacity: an, child: SlideTransition(position: Tween(begin: const Offset(0, 0.25), end: Offset.zero).animate(an), child: c)),
                      child: Column(
                        key: ValueKey(st.title + st.subtitle),
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(st.title, style: tt.headlineLarge?.copyWith(color: fg)),
                          const SizedBox(height: 4),
                          Text(st.subtitle, style: tt.bodyMedium?.copyWith(color: fg.withValues(alpha: 0.85))),
                        ],
                      ),
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 16),
              Wrap(
                spacing: 8,
                runSpacing: 8,
                children: [
                  _HeroChip(icon: viaIcon, label: viaLabel, color: fg),
                  _HeroChip(
                    icon: locked ? Icons.lock_rounded : Icons.lock_open_rounded,
                    label: locked ? 'Locked' : (st.kind == PcKind.nearby || st.kind == PcKind.away ? 'Unlocked' : 'Unknown'),
                    color: fg,
                  ),
                ],
              ),
              const SizedBox(height: 20),
              if (m.extraPhone)
                Text('Extra phone: share, continue and control. Your main phone does the locking and unlocking.', key: const ValueKey('extra-note'), style: TextStyle(color: fg.withValues(alpha: 0.75), fontSize: 13.5, height: 1.35))
              else
                Row(
                  children: [
                    Expanded(flex: 3, child: locked ? unlockBtn : lockBtn),
                    const SizedBox(width: 10),
                    Expanded(flex: 2, child: locked ? lockBtn : unlockBtn),
                  ],
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// The status icon in a morphing blob: squircle when idle, rounder while live.
class _StateBadge extends StatelessWidget {
  const _StateBadge({required this.icon, required this.color, required this.on, required this.live});
  final IconData icon;
  final Color color;
  final Color on;
  final bool live;
  @override
  Widget build(BuildContext context) => AnimatedContainer(
    duration: const Duration(milliseconds: 600),
    curve: Springs.curve,
    width: 84,
    height: 84,
    decoration: BoxDecoration(color: color, borderRadius: BorderRadius.circular(live ? 42 : 30)),
    child: AnimatedSwitcher(
      duration: const Duration(milliseconds: 380),
      switchInCurve: Springs.curve,
      transitionBuilder: (c, an) => ScaleTransition(scale: an, child: FadeTransition(opacity: an, child: c)),
      child: Icon(icon, key: ValueKey(icon), size: 40, color: on),
    ),
  );
}

class _HeroChip extends StatelessWidget {
  const _HeroChip({required this.icon, required this.label, required this.color});
  final IconData icon;
  final String label;
  final Color color;
  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.fromLTRB(10, 7, 14, 7),
    decoration: BoxDecoration(color: color.withValues(alpha: 0.14), borderRadius: BorderRadius.circular(99)),
    child: Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Icon(icon, size: 17, color: color),
        const SizedBox(width: 6),
        Text(label, style: TextStyle(color: color, fontSize: 13.5, fontWeight: FontWeight.w700)),
      ],
    ),
  );
}

const _tlbr = BorderRadius.only(topLeft: Radius.circular(40), topRight: Radius.circular(14), bottomLeft: Radius.circular(14), bottomRight: Radius.circular(40));
const _trbl = BorderRadius.only(topLeft: Radius.circular(14), topRight: Radius.circular(40), bottomLeft: Radius.circular(40), bottomRight: Radius.circular(14));

/// Wake, Sleep, Show laptop, Touchpad, Send file, Ring: six tiles in rotating shapes and tones.
class _ExGrid extends StatelessWidget {
  const _ExGrid({required this.m});
  final AuraModel m;

  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    final tiles = <_ExTile>[
      _ExTile(key: const ValueKey('pc-wake'), icon: Icons.power_settings_new_rounded, label: 'Wake', busy: m.busy.contains('wake'), onTap: m.wakePc, bg: cs.primaryContainer, fg: cs.onPrimaryContainer, shape: _tlbr),
      _ExTile(key: const ValueKey('pc-sleep'), icon: Icons.bedtime_rounded, label: 'Sleep', busy: m.busy.contains('power-sleep'), onTap: () => m.pcPower('sleep'), bg: cs.secondaryContainer, fg: cs.onSecondaryContainer, shape: BorderRadius.circular(34)),
      _ExTile(key: const ValueKey('open-laptop'), icon: Icons.desktop_windows_rounded, label: 'Show laptop', busy: m.busy.contains('laptop'), onTap: m.openLaptop, bg: cs.tertiaryContainer, fg: cs.onTertiaryContainer, shape: _trbl),
      _ExTile(
        key: const ValueKey('touchpad'),
        icon: Icons.mouse_rounded,
        label: 'Touchpad',
        cue: Cue.tap,
        onTap: () => Navigator.of(context).push(auraRoute(const RemoteScreen(initialTab: 'mouse'))),
        bg: cs.secondaryContainer,
        fg: cs.onSecondaryContainer,
        shape: _tlbr,
      ),
      _ExTile(key: const ValueKey('send-file'), icon: Icons.upload_file_rounded, label: 'Send file', cue: Cue.tap, busy: m.busy.contains('file'), onTap: m.sendFile, bg: cs.tertiaryContainer, fg: cs.onTertiaryContainer, shape: BorderRadius.circular(34)),
      _ExTile(key: const ValueKey('remote-ring'), icon: Icons.notifications_active_rounded, label: 'Ring', busy: m.busy.contains('ring'), onTap: () => m.ringLaptop(true), bg: cs.primaryContainer, fg: cs.onPrimaryContainer, shape: _trbl),
    ];
    return Column(
      children: [
        for (var r = 0; r < 2; r++) ...[
          if (r > 0) const SizedBox(height: 10),
          Row(
            children: [
              for (var c = 0; c < 3; c++) ...[
                if (c > 0) const SizedBox(width: 10),
                Expanded(child: tiles[r * 3 + c]),
              ],
            ],
          ),
        ],
      ],
    );
  }
}

class _ExTile extends StatelessWidget {
  const _ExTile({super.key, required this.icon, required this.label, required this.onTap, required this.bg, required this.fg, required this.shape, this.busy = false, this.cue});
  final IconData icon;
  final String label;
  final VoidCallback onTap;
  final Color bg;
  final Color fg;
  final BorderRadius shape;
  final bool busy;

  /// null = the action plays its own cue (send / approved ...).
  final Cue? cue;

  @override
  Widget build(BuildContext context) => Pressable(
    onTap: busy ? null : onTap,
    cue: cue,
    scale: 0.92,
    semanticLabel: label,
    child: AnimatedContainer(
      duration: const Duration(milliseconds: 500),
      curve: Springs.curve,
      height: 104,
      decoration: BoxDecoration(color: bg, borderRadius: shape),
      padding: const EdgeInsets.symmetric(horizontal: 6),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.center,
        children: [
          SizedBox.square(
            dimension: 32,
            child: busy ? Padding(padding: const EdgeInsets.all(5), child: CircularProgressIndicator(strokeWidth: 2.6, color: fg)) : Icon(icon, size: 30, color: fg),
          ),
          const SizedBox(height: 8),
          FittedBox(fit: BoxFit.scaleDown, child: Text(label, maxLines: 1, style: TextStyle(color: fg, fontSize: 14.5, fontWeight: FontWeight.w700))),
        ],
      ),
    ),
  );
}

/// A wide tonal pill: icon + label (Continue on PC, Clipboard to PC, Music & slides).
class _ExWide extends StatelessWidget {
  const _ExWide({super.key, required this.icon, required this.label, required this.onTap, this.busy = false, this.cue});
  final IconData icon;
  final String label;
  final VoidCallback onTap;
  final bool busy;
  final Cue? cue;
  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    return Pressable(
      onTap: busy ? null : onTap,
      cue: cue,
      semanticLabel: label,
      scale: 0.96,
      child: Container(
        height: 64,
        padding: const EdgeInsets.symmetric(horizontal: 16),
        decoration: BoxDecoration(color: cs.surfaceContainerHigh, borderRadius: BorderRadius.circular(99)),
        child: Row(
          children: [
            busy ? SizedBox.square(dimension: 22, child: CircularProgressIndicator(strokeWidth: 2.4, color: cs.primary)) : Icon(icon, color: cs.primary, size: 24),
            const SizedBox(width: 10),
            Expanded(child: Text(label, maxLines: 1, overflow: TextOverflow.ellipsis, style: TextStyle(color: cs.onSurface, fontSize: 15, fontWeight: FontWeight.w700))),
          ],
        ),
      ),
    );
  }
}

/// Connected devices with live status chips: the PC (link + lock) and the Galaxy Watch.
class _ExDevices extends StatelessWidget {
  const _ExDevices({required this.m});
  final AuraModel m;
  @override
  Widget build(BuildContext context) {
    final st = m.status;
    final via = viaOf((m.s['linkVia'] as String?) ?? '');
    final pinned = ((m.s['watchPinnedFp'] as String?) ?? '').isNotEmpty;
    final pend = ((m.s['watchPendingFp'] as String?) ?? '').isNotEmpty;
    final locked = st.kind == PcKind.locked || st.kind == PcKind.unlocking;
    final linkLabel = switch (via) {
      Via.bluetooth => 'Bluetooth',
      Via.lan => 'Wi-Fi',
      Via.tailnet => 'Tailscale',
      Via.none => st.kind == PcKind.outOfReach ? 'Offline' : 'Standby',
    };
    final linkIcon = switch (via) {
      Via.bluetooth => Icons.bluetooth_rounded,
      Via.lan => Icons.wifi_rounded,
      Via.tailnet => Icons.vpn_lock_rounded,
      Via.none => Icons.link_off_rounded,
    };
    final connected = via != Via.none;
    return IntrinsicHeight(
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Expanded(
            child: _DeviceCard(
              key: const ValueKey('device-pc'),
              icon: Icons.laptop_windows_rounded,
              title: 'Laptop',
              onTap: m.askPc,
              chips: [
                Pill(linkLabel, tone: connected ? Tone.good : (st.kind == PcKind.outOfReach ? Tone.warn : Tone.neutral), icon: linkIcon),
                if (st.kind != PcKind.unpaired && st.kind != PcKind.outOfReach)
                  Pill(locked ? 'Locked' : 'Unlocked', tone: locked ? Tone.info : Tone.good, icon: locked ? Icons.lock_rounded : Icons.lock_open_rounded),
              ],
            ),
          ),
          const SizedBox(width: 10),
          Expanded(
            child: _DeviceCard(
              key: const ValueKey('device-watch'),
              icon: Icons.watch_rounded,
              title: 'Watch',
              onTap: () => Navigator.of(context).push(auraRoute(const SettingsScreen())),
              chips: [
                Pill(pend ? 'Wants to pair' : (pinned ? 'Paired' : 'Not paired'), tone: pend ? Tone.warn : (pinned ? Tone.good : Tone.neutral), icon: pinned ? Icons.check_circle_rounded : Icons.watch_off_rounded),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _DeviceCard extends StatelessWidget {
  const _DeviceCard({super.key, required this.icon, required this.title, required this.chips, required this.onTap});
  final IconData icon;
  final String title;
  final List<Widget> chips;
  final VoidCallback onTap;
  @override
  Widget build(BuildContext context) {
    final cs = Theme.of(context).colorScheme;
    return Pressable(
      onTap: onTap,
      semanticLabel: title,
      scale: 0.97,
      child: Container(
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(color: cs.surfaceContainerLow, borderRadius: BorderRadius.circular(30), border: Border.all(color: cs.outlineVariant.withValues(alpha: 0.5))),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                IconTile(icon),
                const SizedBox(width: 10),
                Expanded(child: Text(title, style: Theme.of(context).textTheme.titleMedium?.copyWith(color: cs.onSurface))),
              ],
            ),
            const SizedBox(height: 12),
            Wrap(spacing: 6, runSpacing: 6, children: chips),
          ],
        ),
      ),
    );
  }
}
