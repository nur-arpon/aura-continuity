// Settings: everything the old screen had (approval mode, companion link, watch, Handoff,
// away mode, battery, test mode, listener start / stop, log, forget PC) plus Feel (haptic
// intensity + sounds), grouped and explained in plain words.
import 'package:flutter/material.dart';

import '../core/fx.dart';
import '../core/model.dart';
import '../core/motion.dart';
import '../core/theme.dart';
import '../core/widgets.dart';
import '../core/pair_qr.dart';
import 'inbox.dart' show PageHeader;
import 'scan_qr.dart';
import 'remote.dart';
import 'unlock_guide.dart';
import 'update_row.dart';

/// A segmented control with a sliding thumb on a spring, and a tick per change.
class AuraSegmented<T> extends StatelessWidget {
  const AuraSegmented({super.key, required this.values, required this.labels, required this.value, required this.onChanged});
  final List<T> values;
  final List<String> labels;
  final T value;
  final ValueChanged<T> onChanged;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final idx = values.indexOf(value).clamp(0, values.length - 1);
    if (a.expressive) {
      // Expressive: the real Material 3 segmented button (pill shaped), with a tick per change.
      return SizedBox(
        width: double.infinity,
        child: SegmentedButton<T>(
          showSelectedIcon: false,
          style: SegmentedButton.styleFrom(minimumSize: const Size(0, 48), padding: const EdgeInsets.symmetric(horizontal: 6)),
          segments: [
            for (var i = 0; i < values.length; i++)
              ButtonSegment<T>(value: values[i], label: KeyedSubtree(key: ValueKey('seg-${labels[i]}'), child: Text(labels[i], maxLines: 1, overflow: TextOverflow.fade, softWrap: false))),
          ],
          selected: {values[idx]},
          onSelectionChanged: (v) {
            if (v.first == value) return;
            Fx.tick();
            onChanged(v.first);
          },
        ),
      );
    }
    return LayoutBuilder(
      builder: (context, c) {
        final w = (c.maxWidth - 8) / values.length;
        return Container(
          height: 44,
          padding: const EdgeInsets.all(4),
          decoration: BoxDecoration(
            color: a.surface3.withValues(alpha: a.dark ? 0.8 : 1),
            borderRadius: BorderRadius.circular(14),
          ),
          child: Stack(
            children: [
              AnimatedPositioned(
                duration: const Duration(milliseconds: 420),
                curve: Springs.curve,
                left: idx * w,
                top: 0,
                bottom: 0,
                width: w,
                child: Container(
                  decoration: BoxDecoration(
                    color: a.dark ? a.primary : a.surface1,
                    borderRadius: BorderRadius.circular(11),
                    boxShadow: [
                      BoxShadow(
                        color: Colors.black.withValues(alpha: a.dark ? 0.25 : 0.10),
                        blurRadius: 8,
                        offset: const Offset(0, 2),
                      ),
                    ],
                  ),
                ),
              ),
              Row(
                children: [
                  for (var i = 0; i < values.length; i++)
                    Expanded(
                      child: GestureDetector(
                        key: ValueKey('seg-${labels[i]}'),
                        behavior: HitTestBehavior.opaque,
                        onTap: () {
                          if (values[i] == value) return;
                          Fx.tick();
                          onChanged(values[i]);
                        },
                        child: Center(
                          child: AnimatedDefaultTextStyle(
                            duration: const Duration(milliseconds: 200),
                            style: TextStyle(
                              color: i == idx ? (a.dark ? a.onPrimary : a.text) : a.text2,
                              fontSize: 13.5,
                              fontWeight: i == idx ? FontWeight.w700 : FontWeight.w500,
                              fontFamily: 'Roboto',
                            ),
                            child: Text(labels[i]),
                          ),
                        ),
                      ),
                    ),
                ],
              ),
            ],
          ),
        );
      },
    );
  }
}

class SettingsScreen extends StatelessWidget {
  const SettingsScreen({super.key});

  Future<void> _addLaptop(BuildContext context, AuraModel m) async {
    Fx.tap();
    final code = await scanPairQr(context);
    final q = PairQr.last;
    if (code == null || q == null) return;
    final ok = await m.addLaptop(code, q);
    if (!context.mounted) return;
    ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(ok ? 'Laptop added.' : 'That code did not work. Make a new one in Aura on the laptop.')));
  }

  Future<void> _forget(BuildContext context, AuraModel m) async {
    Fx.tap();
    final a = context.aura;
    final ok = await showDialog<bool>(
      context: context,
      builder: (c) => AlertDialog(
        icon: Icon(Icons.link_off_rounded, color: a.danger),
        title: Text('Forget ${m.pcName == 'Your PC' ? 'this PC' : m.pcName}?'),
        content: const Text('This phone stops unlocking, locking and handing off with it. To use Aura again, pair again with a new code from the PC.'),
        actions: [
          TextButton(key: const ValueKey('forget-cancel'), onPressed: () { Fx.tap(); Navigator.pop(c, false); }, child: const Text('Keep')),
          TextButton(
            key: const ValueKey('forget-confirm'),
            onPressed: () { Fx.tap(); Navigator.pop(c, true); },
            child: Text(
              'Forget',
              style: TextStyle(color: a.danger, fontWeight: FontWeight.w700),
            ),
          ),
        ],
      ),
    );
    if (ok == true) {
      await m.forgetPc();
      if (context.mounted) Navigator.of(context).popUntil((r) => r.isFirst);
    }
  }

  /// Item 27: which installed app opens YouTube links (Automatic = system default, else the first YouTube app).
  Future<void> _chooseYoutube(BuildContext context, AuraModel m, Map<String, dynamic> h) async {
    Fx.tap();
    final apps = (h['ytHandlers'] as List).whereType<Map>().toList();
    final chosen = '${h['ytChosen'] ?? ''}';
    final pick = await showModalBottomSheet<String>(
      context: context,
      builder: (c) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              key: const ValueKey('yt-auto'),
              leading: const Icon(Icons.auto_awesome_rounded),
              title: const Text('Automatic'),
              subtitle: const Text('The phone default, else the first YouTube app'),
              trailing: chosen.isEmpty ? const Icon(Icons.check_rounded) : null,
              onTap: () { Fx.tap(); Navigator.pop(c, ''); },
            ),
            for (final app in apps)
              ListTile(
                key: ValueKey('yt-${app['pkg']}'),
                leading: Icon(app['browser'] == true ? Icons.public_rounded : Icons.smart_display_rounded),
                title: Text('${app['label']}'),
                trailing: chosen == app['pkg'] ? const Icon(Icons.check_rounded) : null,
                onTap: () { Fx.tap(); Navigator.pop(c, '${app['pkg']}'); },
              ),
          ],
        ),
      ),
    );
    if (pick != null) await m.set('setYtHandler', {'pkg': pick});
  }

  void _log(BuildContext context, AuraModel m) {
    Fx.tap();
    final log = ((m.s['log'] as List?) ?? const []).cast<String>();
    final a = context.aura;
    showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      builder: (c) => SizedBox(
        height: MediaQuery.sizeOf(c).height * 0.7,
        child: ListView(
          padding: const EdgeInsets.fromLTRB(18, 0, 18, 24),
          children: [
            Text(
              'Log',
              style: TextStyle(color: a.text, fontSize: 20, fontWeight: FontWeight.w700),
            ),
            const SizedBox(height: 10),
            for (final l in log)
              Text(
                l,
                style: TextStyle(fontFamily: 'monospace', fontSize: 11.5, color: a.text2, height: 1.45),
              ),
          ],
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final m = AuraScope.of(context);
    final a = context.aura;
    final s = m.s;
    final h = m.h;
    final mode = (s['mode'] as String?) ?? 'auto';
    final level = (s['hapticLevel'] as int?) ?? 3;
    final pinned = (s['watchPinnedFp'] as String?) ?? '';
    final pend = (s['watchPendingFp'] as String?) ?? '';
    final missing = ((s['missingPerms'] as List?) ?? const []).cast<String>();
    final away = (m.a['code'] as String?) ?? '';
    final awayAction = (m.a['action'] as String?) ?? 'none';
    var i = 0;
    return Scaffold(
      body: AuraBackground(
        child: SafeArea(
          bottom: false,
          child: ListView(
            key: const ValueKey('settings-list'),
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 48),
            children: [
              const PageHeader(title: 'Settings'),
              const UpdateRow(rowKey: ValueKey('update-settings')),
              // ------------------------------------------------ PC
              const SectionLabel('Your PC'),
              Entrance(
                index: i++,
                child: Group(
                  children: [
                    Row2(icon: Icons.laptop_windows_rounded, title: m.pcName, subtitle: '${m.status.title} · ${m.status.subtitle}', tone: m.status.tone),
                    if (s['companionSupported'] == true)
                      Row2(
                        rowKey: const ValueKey('companion'),
                        icon: s['companion'] == true ? Icons.bolt_rounded : Icons.link_rounded,
                        tone: s['companion'] == true ? Tone.good : Tone.warn,
                        title: 'Always ready',
                        subtitle: s['companion'] == true
                            ? 'Starts by itself after a restart, an update, or when your PC connects. No polling.'
                            : ((s['pcAddr'] as String?) ?? '').isEmpty
                            ? 'Unlock once from the PC first, then link it here.'
                            : 'One tap: Android shows a sheet, pick your PC.',
                        trailing: s['companion'] == true
                            ? Icon(Icons.check_circle_rounded, color: a.success)
                            : PillButton('Link', cue: null, onTap: m.linkCompanion),
                        onTap: s['companion'] == true ? null : m.linkCompanion,
                      ),
                    Row2(
                      rowKey: const ValueKey('battery'),
                      icon: Icons.battery_charging_full_rounded,
                      tone: s['batteryUnrestricted'] == true ? Tone.good : Tone.warn,
                      title: 'Battery',
                      subtitle: s['batteryUnrestricted'] == true
                          ? 'Unrestricted. Aura sleeps until your PC needs it.'
                          : 'Set Aura to Unrestricted so it is never put to sleep.',
                      trailing: s['batteryUnrestricted'] == true
                          ? Icon(Icons.check_circle_rounded, color: a.success)
                          : PillButton('Open', cue: null, onTap: () => m.bridge.call('batterySettings')),
                      onTap: s['batteryUnrestricted'] == true ? null : () => m.bridge.call('batterySettings'),
                    ),
                    if (missing.isNotEmpty)
                      Row2(
                        rowKey: const ValueKey('perms'),
                        icon: Icons.bluetooth_searching_rounded,
                        tone: Tone.warn,
                        title: 'Permissions needed',
                        subtitle: missing.map((p) => p.split('.').last.replaceAll('_', ' ').toLowerCase()).join(', '),
                        trailing: PillButton('Allow', cue: null, onTap: m.requestPerms),
                        onTap: m.requestPerms,
                      ),
                  ],
                ),
              ),
              // ------------------------------------------------ unlocking
              const SectionLabel('Unlocking your PC'),
              Entrance(
                index: i++,
                child: AuraCard(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        'When your PC asks',
                        style: TextStyle(color: a.text, fontSize: 15.5, fontWeight: FontWeight.w600),
                      ),
                      const SizedBox(height: 10),
                      AuraSegmented<String>(
                        key: const ValueKey('mode'),
                        values: const ['auto', 'tap', 'fingerprint'],
                        labels: const ['Automatic', 'Tap', 'Fingerprint'],
                        value: mode,
                        onChanged: (v) => m.set('setMode', {'mode': v}),
                      ),
                      const SizedBox(height: 10),
                      Text(switch (mode) {
                        'tap' => 'You approve each unlock with one tap on this phone.',
                        'fingerprint' => 'You approve each unlock with your fingerprint.',
                        _ => 'Unlocks with no tap while this phone is unlocked (or was, in the last 5 min) and close to the PC. Otherwise it asks for your fingerprint.',
                      }, style: TextStyle(color: a.text3, fontSize: 12.8, height: 1.4)),
                    ],
                  ),
                ),
              ),
              Entrance(
                index: i++,
                child: Padding(
                  padding: const EdgeInsets.only(top: 12),
                  child: Group(
                    children: [
                      Row2(
                        rowKey: const ValueKey('stay-unlocked'),
                        icon: Icons.phonelink_lock_rounded,
                        title: 'Stay unlocked near my laptop',
                        subtitle: 'How to add your laptop as a trusted Bluetooth device',
                        onTap: () => Navigator.of(context).push(auraRoute(const UnlockGuideScreen())),
                        trailing: Icon(Icons.chevron_right_rounded, color: a.text3),
                      ),
                      Row2(
                        rowKey: const ValueKey('open-remote'),
                        icon: Icons.settings_remote_rounded,
                        title: 'Remote and find',
                        subtitle: 'Music, slides, ring my laptop, let my PC ring this phone',
                        onTap: () => Navigator.of(context).push(auraRoute(const RemoteScreen(initialTab: 'find'))),
                        trailing: Icon(Icons.chevron_right_rounded, color: a.text3),
                      ),
                    ],
                  ),
                ),
              ),
              // ------------------------------------------------ appearance
              const SectionLabel('Appearance'),
              Entrance(index: i++, child: _Appearance(m: m)),
              // ------------------------------------------------ feel
              const SectionLabel('Feel'),
              Entrance(
                index: i++,
                child: AuraCard(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          const IconTile(Icons.vibration_rounded),
                          const SizedBox(width: 12),
                          Expanded(
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  'Haptics',
                                  style: TextStyle(color: a.text, fontSize: 15.5, fontWeight: FontWeight.w600),
                                ),
                                Text('Every action has its own feel', style: TextStyle(color: a.text3, fontSize: 12.8)),
                              ],
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 12),
                      AuraSegmented<int>(
                        key: const ValueKey('haptics'),
                        values: const [0, 1, 2, 3],
                        labels: const ['Off', 'Light', 'Medium', 'Max'],
                        value: level,
                        onChanged: m.setHaptics,
                      ),
                    ],
                  ),
                ),
              ),
              const SizedBox(height: 12),
              Entrance(
                index: i++,
                child: Group(
                  children: [
                    SwitchRow(
                      switchKey: const ValueKey('sound'),
                      icon: Icons.graphic_eq_rounded,
                      title: 'Sounds',
                      subtitle: 'Short Aura sounds. Silent in silent mode and Do Not Disturb.',
                      value: s['sound'] != false,
                      onChanged: m.setSound,
                    ),
                  ],
                ),
              ),
              // ------------------------------------------------ handoff
              const SectionLabel('Handoff'),
              Entrance(
                index: i++,
                child: Group(
                  children: [
                    Row2(
                      rowKey: const ValueKey('media-access'),
                      icon: Icons.smart_display_rounded,
                      tone: h['mediaAccess'] == true ? Tone.good : Tone.info,
                      title: 'YouTube at the exact second',
                      subtitle: h['mediaAccess'] == true ? 'Media access is on.' : 'Needs media access. Aura explains it first.',
                      trailing: h['mediaAccess'] == true
                          ? Icon(Icons.check_circle_rounded, color: a.success)
                          : PillButton('Set up', cue: null, onTap: () => m.bridge.call('mediaAccess')),
                      onTap: h['mediaAccess'] == true ? null : () => m.bridge.call('mediaAccess'),
                    ),
                    SwitchRow(
                      switchKey: const ValueKey('suggest'),
                      icon: Icons.auto_awesome_rounded,
                      title: 'Suggest Continue on phone / PC',
                      subtitle: 'A quiet notification when your PC shows something you can carry on.',
                      value: h['suggest'] != false,
                      onChanged: (v) => m.set('setSuggest', {'on': v}),
                    ),
                    SwitchRow(
                      switchKey: const ValueKey('chathead'),
                      icon: Icons.bubble_chart_rounded,
                      title: 'Chat head for your laptop',
                      subtitle: "An Aura bubble over your apps. Tap it for your laptop's recent apps, tap a browser for its tabs, tap a tab to open it here. It hides during calls and full-screen apps.",
                      value: h['chatHead'] != false,
                      onChanged: (v) => m.set('setChatHead', {'on': v}),
                    ),
                    if (h['chatHead'] != false && h['chatHeadAllowed'] == false)
                      Row2(
                        rowKey: const ValueKey('chathead-allow'),
                        icon: Icons.layers_rounded,
                        tone: Tone.warn,
                        title: 'Allow display over other apps',
                        subtitle: 'Android needs this once so the Aura head can float over other apps. Tap to turn it on.',
                        trailing: PillButton('Open', cue: null, onTap: () => m.bridge.call('chatHeadSettings')),
                        onTap: () => m.bridge.call('chatHeadSettings'),
                      ),
                    if (h['bubbleSupported'] == true) ...[
                      SwitchRow(
                        switchKey: const ValueKey('bubble'),
                        icon: Icons.bubble_chart_rounded,
                        title: 'Bubble for Continue on phone',
                        subtitle: 'A floating bubble when your PC pushes something new. Tap it to see everything from your PC. It goes away by itself after 2 minutes.',
                        value: h['bubble'] != false,
                        onChanged: (v) => m.set('setBubble', {'on': v}),
                      ),
                      if (h['bubble'] != false && h['bubbleAllowed'] == false)
                        Row2(
                          rowKey: const ValueKey('bubble-allow'),
                          icon: Icons.settings_rounded,
                          tone: Tone.warn,
                          title: 'Allow bubbles for Aura',
                          subtitle: (h['bubbleBlocked'] as String?)?.isNotEmpty == true ? '${h['bubbleBlocked']}. Tap to turn them on.' : 'Android has bubbles turned off for Aura. Turn them on in its notification settings.',
                          trailing: PillButton('Open', cue: null, onTap: () => m.bridge.call('bubbleSettings')),
                          onTap: () => m.bridge.call('bubbleSettings'),
                        )
                      else if (h['bubble'] != false)
                        Row2(
                          rowKey: const ValueKey('bubble-test'),
                          icon: Icons.bubble_chart_rounded,
                          title: 'Send a test bubble',
                          subtitle: 'Shows a bubble now from what your PC last sent.',
                          trailing: PillButton('Test', cue: null, onTap: () => m.bridge.call('testBubble')),
                          onTap: () => m.bridge.call('testBubble'),
                        ),
                    ],
                    SwitchRow(
                      switchKey: const ValueKey('clipsync'),
                      icon: Icons.content_paste_rounded,
                      title: 'Share the clipboard',
                      subtitle:
                          'What you copy on the PC lands here. From the phone, use Clipboard to PC (Android lets only the app in front read the clipboard).',
                      value: h['clipSync'] == true,
                      onChanged: (v) => m.set('setClipSync', {'on': v}),
                    ),
                  ],
                ),
              ),
              // ------------------------------------------------ links (item 24)
              if (h['linkApps'] is List) ...[
                const SectionLabel('Open links from your PC'),
                Entrance(
                  index: i++,
                  child: Group(
                    children: [
                      for (final app in (h['linkApps'] as List).whereType<Map>())
                        Row2(
                          rowKey: ValueKey('link-${app['key']}'),
                          icon: Icons.open_in_new_rounded,
                          title: '${app['label']}',
                          subtitle: app['mode'] == 'browser' ? 'Opens in the browser.' : 'Opens in the app if you have it, else the browser.',
                          trailing: PillButton(
                            app['mode'] == 'browser' ? 'Browser' : 'App',
                            cue: null,
                            onTap: () => m.set('setLinkMode', {'app': app['key'], 'mode': app['mode'] == 'browser' ? 'app' : 'browser'}),
                          ),
                          onTap: () => m.set('setLinkMode', {'app': app['key'], 'mode': app['mode'] == 'browser' ? 'app' : 'browser'}),
                        ),
                    ],
                  ),
                ),
                if (h['ytHandlers'] is List && (h['ytHandlers'] as List).isNotEmpty) ...[
                  const SizedBox(height: 10),
                  Group(
                    children: [
                      Row2(
                        rowKey: const ValueKey('yt-opens-with'),
                        icon: Icons.smart_display_rounded,
                        title: 'YouTube opens with',
                        subtitle: h['ytChosen'] == '' || h['ytChosen'] == null
                            ? 'Automatic: ${h['ytEffective'] ?? 'Browser'}'
                            : '${h['ytEffective'] ?? ''}',
                        trailing: const Icon(Icons.expand_more_rounded, size: 20),
                        onTap: () => _chooseYoutube(context, m, h),
                      ),
                    ],
                  ),
                ],
                Padding(
                  padding: const EdgeInsets.fromLTRB(20, 6, 20, 0),
                  child: Text('Long-press a link under More from your PC to choose just once.', style: TextStyle(color: a.text3, fontSize: 12.5)),
                ),
              ],
              // ------------------------------------------------ zero-tap continue (item 19)
              if (s['play'] != true) const SectionLabel('Zero-tap continue'),
              if (s['play'] != true) Entrance(
                index: i++,
                child: Group(
                  children: [
                    SwitchRow(
                      switchKey: const ValueKey('zt-on'),
                      icon: Icons.bolt_rounded,
                      title: 'Zero-tap continue',
                      subtitle: 'Open YouTube, your browser, Facebook, Messenger, WhatsApp or Claude and it carries on what your PC showed, at the same second. Undo for 2 s.',
                      value: h['zt_on'] != false,
                      onChanged: (v) => m.set('zeroTapSet', {'key': 'on', 'on': v}),
                    ),
                    if (h['zt_on'] != false) ...[
                      SwitchRow(
                        switchKey: const ValueKey('zt-pc-phone'),
                        icon: Icons.phone_android_rounded,
                        title: 'PC to phone',
                        subtitle: 'When you open one of those apps here.',
                        value: h['zt_pcToPhone'] != false,
                        onChanged: (v) => m.set('zeroTapSet', {'key': 'pc_to_phone', 'on': v}),
                      ),
                      SwitchRow(
                        switchKey: const ValueKey('zt-phone-pc'),
                        icon: Icons.laptop_chromebook_rounded,
                        title: 'Phone to PC',
                        subtitle: 'When you focus Brave, WhatsApp or Claude on the PC.',
                        value: h['zt_phoneToPc'] != false,
                        onChanged: (v) => m.set('zeroTapSet', {'key': 'phone_to_pc', 'on': v}),
                      ),
                      Padding(
                        padding: const EdgeInsets.fromLTRB(16, 8, 16, 12),
                        child: AuraSegmented<String>(
                          values: const ['pause', 'mute', 'leave'],
                          labels: const ['Pause PC', 'Mute', 'Leave playing'],
                          value: '${h['zt_after'] ?? 'pause'}',
                          onChanged: (v) => m.set('zeroTapAfter', {'v': v}),
                        ),
                      ),
                      Row2(
                        rowKey: const ValueKey('zt-setup'),
                        icon: Icons.accessibility_new_rounded,
                        tone: h['zt_a11y'] == true ? Tone.good : Tone.info,
                        title: 'Know what is on screen',
                        subtitle: h['zt_a11y'] == true
                            ? 'On, for your apps only. Nothing stored.'
                            : (h['zt_usage'] == true ? 'Usage access on. Accessibility adds chats and pages.' : 'Needs Accessibility or Usage access. Aura explains it first.'),
                        trailing: h['zt_a11y'] == true
                            ? Icon(Icons.check_circle_rounded, color: a.success)
                            : PillButton('Set up', cue: null, onTap: () => m.bridge.call('zeroTapSetup')),
                        onTap: () => m.bridge.call('zeroTapSetup'),
                      ),
                    ],
                  ],
                ),
              ),
              // ------------------------------------------------ away
              const SectionLabel('Away mode'),
              Entrance(
                index: i++,
                child: Group(
                  children: [
                    Row2(
                      rowKey: const ValueKey('away'),
                      icon: away == 'ready' ? Icons.public_rounded : Icons.public_off_rounded,
                      tone: away == 'ready' ? Tone.good : Tone.warn,
                      title: (m.a['title'] as String?) ?? 'Away mode',
                      subtitle: (m.a['text'] as String?) ?? '',
                      trailing: awayAction == 'none'
                          ? null
                          : PillButton(awayAction == 'install' ? 'Install' : (away == 'off' ? 'Turn on' : 'Open'), onTap: () => m.bridge.call('tailscale')),
                      onTap: awayAction == 'none' ? null : () => m.bridge.call('tailscale'),
                    ),
                    if (awayAction != 'install' && away != 'unpaired')
                      Row2(
                        rowKey: const ValueKey('away-vpn'),
                        icon: Icons.vpn_lock_rounded,
                        title: 'Keep Tailscale always on',
                        subtitle: 'In Android VPN settings, tap the gear next to Tailscale and turn on Always-on VPN, so Continue works on mobile data without opening Tailscale.',
                        trailing: PillButton('Open', cue: null, onTap: () => m.bridge.call('vpnSettings')),
                        onTap: () => m.bridge.call('vpnSettings'),
                      ),
                    if (awayAction != 'install' && away != 'unpaired')
                      SwitchRow(
                        switchKey: const ValueKey('ts-auto-off'),
                        icon: Icons.power_settings_new_rounded,
                        title: 'Turn Tailscale off after streaming',
                        subtitle: 'When Aura switched Tailscale on to open your laptop, it switches it off again 2 minutes after you leave Artemis. Never if you turned Tailscale on yourself.',
                        value: m.a['tsAutoOff'] != false,
                        onChanged: (v) => m.set('setTsAutoOff', {'on': v}),
                      ),
                    Row2(
                      icon: Icons.shield_rounded,
                      title: 'Unlock PC from away',
                      subtitle: 'Asks for your fingerprint every time and only approves the sign-in screen showing on the PC. Never automatic.',
                    ),
                  ],
                ),
              ),
              // ------------------------------------------------ laptops (many devices)
              const SectionLabel('Laptops'),
              Entrance(
                index: i++,
                child: Group(
                  children: [
                    for (final l in ((m.s['laptops'] as List?) ?? const []).cast<Map>())
                      Row2(
                        rowKey: ValueKey('laptop-${l['id']}'),
                        icon: Icons.laptop_rounded,
                        tone: l['connected'] == true ? Tone.good : Tone.info,
                        title: '${l['name']}${(m.s['laptopPin'] ?? '') == l['id'] ? ' (always)' : ''}',
                        subtitle: (l['primary'] == true && !m.extraPhone ? 'Main laptop: unlocks with this phone. ' : 'Companion: Continue, clipboard, files, remote. ') + (l['connected'] == true ? 'Connected.' : 'Not connected.'),
                        trailing: l['primary'] == true ? null : PillButton('Remove', cue: Cue.toggleOff, onTap: () => m.set('laptopRemove', {'id': l['id']})),
                        onTap: () => m.set('laptopPin', {'id': (m.s['laptopPin'] ?? '') == l['id'] ? '' : l['id']}),
                      ),
                    Row2(
                      rowKey: const ValueKey('laptop-add'),
                      icon: Icons.add_link_rounded,
                      title: 'Add a laptop',
                      subtitle: 'Scan the QR from Aura on the other laptop (Devices > Add another phone). Tap a laptop to always use it; tap again for automatic (the nearest one).',
                      trailing: PillButton('Scan QR', cue: null, onTap: () => _addLaptop(context, m)),
                      onTap: () => _addLaptop(context, m),
                    ),
                  ],
                ),
              ),
              // ------------------------------------------------ watch
              const SectionLabel('Galaxy Watch'),
              Entrance(
                index: i++,
                child: Group(
                  children: [
                    if (pend.isNotEmpty)
                      Row2(
                        rowKey: const ValueKey('watch-confirm'),
                        icon: Icons.watch_rounded,
                        tone: Tone.warn,
                        title: 'Watch asks to pair: $pend',
                        subtitle: 'Confirm only if the watch shows exactly the same code.',
                        trailing: PillButton('Same code', filled: true, cue: Cue.success, onTap: () => m.set('watchConfirm', const {})),
                      )
                    else
                      Row2(
                        rowKey: const ValueKey('watch'),
                        icon: Icons.watch_rounded,
                        tone: pinned.isNotEmpty ? Tone.good : Tone.info,
                        title: pinned.isNotEmpty ? 'Watch paired' : 'Watch not paired',
                        subtitle: pinned.isNotEmpty
                            ? 'Key $pinned. The watch can lock your PC and unlock it from its own button.'
                            : 'On the watch open Aura and tap Pair with phone.',
                        trailing: pinned.isNotEmpty ? PillButton('Unpair', cue: Cue.toggleOff, onTap: () => m.set('watchUnpair', const {})) : null,
                      ),
                  ],
                ),
              ),
              // ------------------------------------------------ advanced
              const SectionLabel('Advanced'),
              Entrance(
                index: i++,
                child: Group(
                  children: [
                    Row2(
                      icon: Icons.sensors_rounded,
                      tone: s['running'] == true ? Tone.good : Tone.warn,
                      title: s['running'] == true ? 'Listener running' : 'Listener stopped',
                      subtitle: [
                        'answered ${s['answered'] ?? 0}',
                        if (s['rfcomm'] == true) 'Bluetooth',
                        if (s['gatt'] == true) 'BLE',
                        if (s['advertising'] == true) 'advertising',
                        if (((s['usb'] as String?) ?? '').isNotEmpty) 'USB',
                        'self-heal ${s['restarts'] ?? 0}',
                      ].join(' · '),
                      trailing: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          PillButton('Start', key: const ValueKey('start'), cue: Cue.toggleOn, onTap: () => m.set('start', const {})),
                          const SizedBox(width: 6),
                          PillButton('Stop', key: const ValueKey('stop'), cue: Cue.toggleOff, onTap: () => m.set('stop', const {})),
                        ],
                      ),
                    ),
                    SwitchRow(
                      switchKey: const ValueKey('testmode'),
                      icon: Icons.science_rounded,
                      tone: s['auto'] == true ? Tone.warn : Tone.info,
                      title: 'Test mode (30 min)',
                      subtitle: 'Answers at Tap level with no tap, for unattended tests. Switches itself off after 30 minutes.',
                      value: s['auto'] == true,
                      onChanged: (v) => m.set('setAuto', {'on': v}),
                    ),
                    Row2(
                      rowKey: const ValueKey('wake-remember'),
                      icon: Icons.wifi_rounded,
                      title: 'Remember this Wi-Fi for Wake',
                      subtitle: 'Lets Wake PC find your PC on this network. Networks you and the PC share are remembered automatically.',
                      onTap: m.rememberWifiForWake,
                    ),
                    Row2(
                      rowKey: const ValueKey('log'),
                      icon: Icons.receipt_long_rounded,
                      title: 'Log',
                      subtitle: 'What Aura did, newest first. Never contains clipboard text.',
                      trailing: Icon(Icons.chevron_right_rounded, color: a.text3),
                      onTap: () => _log(context, m),
                      cue: null,
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 22),
              Entrance(
                index: i++,
                child: Group(
                  children: [
                    Row2(
                      rowKey: const ValueKey('forget'),
                      icon: Icons.link_off_rounded,
                      tone: Tone.warn,
                      title: 'Forget this PC',
                      subtitle: 'Asks first.',
                      onTap: () => _forget(context, m),
                      cue: null,
                    ),
                  ],
                ),
              ),
              const SizedBox(height: 26),
              Center(
                child: Column(
                  children: [
                    const AuraOrb(size: 44),
                    const SizedBox(height: 6),
                    Text(
                      'Aura ${(s['version'] as String?) ?? ''}',
                      style: TextStyle(color: a.text2, fontSize: 13, fontWeight: FontWeight.w600),
                    ),
                    Text('Continuity for your PC · SpaceZ', style: TextStyle(color: a.text3, fontSize: 12)),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}


/// Settings > Appearance: Classic (the original Aura look) or Expressive (Material 3, the default),
/// and for Expressive either the wallpaper's colours (Material You) or one of eight seed colours.
class _Appearance extends StatelessWidget {
  const _Appearance({required this.m});
  final AuraModel m;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final cs = Theme.of(context).colorScheme;
    final seed = m.seed;
    return Column(
      children: [
        Group(
          children: [
            SwitchRow(
              switchKey: const ValueKey('classic-look'),
              icon: Icons.palette_outlined,
              title: 'Classic look',
              subtitle: m.expressive ? 'Off: Material You, rounded and colourful' : 'On: the original navy Aura theme',
              value: !m.expressive,
              onChanged: (v) => m.setLook(expressive: !v),
            ),
          ],
        ),
        AnimatedSize(
          duration: const Duration(milliseconds: 450),
          curve: Springs.curve,
          alignment: Alignment.topCenter,
          child: !m.expressive
              ? const SizedBox(width: double.infinity)
              : Padding(
                  padding: const EdgeInsets.only(top: 12),
                  child: AuraCard(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text('Colour', style: Theme.of(context).textTheme.titleMedium?.copyWith(color: a.text)),
                        const SizedBox(height: 12),
                        Pressable(
                          key: const ValueKey('seed-wallpaper'),
                          onTap: () => m.setLook(seed: 0),
                          cue: null,
                          semanticLabel: 'Wallpaper colours',
                          scale: 0.97,
                          child: AnimatedContainer(
                            duration: const Duration(milliseconds: 300),
                            height: 52,
                            padding: const EdgeInsets.symmetric(horizontal: 16),
                            decoration: BoxDecoration(
                              color: seed == null ? cs.primaryContainer : cs.surfaceContainerHighest,
                              borderRadius: BorderRadius.circular(99),
                            ),
                            child: Row(
                              children: [
                                Icon(Icons.wallpaper_rounded, color: seed == null ? cs.onPrimaryContainer : cs.onSurfaceVariant),
                                const SizedBox(width: 10),
                                Expanded(
                                  child: Text(
                                    'Wallpaper colours',
                                    style: TextStyle(color: seed == null ? cs.onPrimaryContainer : cs.onSurface, fontWeight: FontWeight.w700, fontSize: 15),
                                  ),
                                ),
                                if (seed == null) Icon(Icons.check_rounded, color: cs.onPrimaryContainer),
                              ],
                            ),
                          ),
                        ),
                        const SizedBox(height: 14),
                        Wrap(
                          spacing: 12,
                          runSpacing: 12,
                          children: [
                            for (final c in auraSeeds)
                              _Swatch(
                                key: ValueKey('seed-${c.toRadixString(16)}'),
                                color: Color(c),
                                selected: seed == c,
                                onTap: () => m.setLook(seed: c),
                              ),
                          ],
                        ),
                      ],
                    ),
                  ),
                ),
        ),
      ],
    );
  }
}

class _Swatch extends StatelessWidget {
  const _Swatch({super.key, required this.color, required this.selected, required this.onTap});
  final Color color;
  final bool selected;
  final VoidCallback onTap;
  @override
  Widget build(BuildContext context) => Pressable(
    onTap: onTap,
    cue: null,
    semanticLabel: 'Seed colour',
    scale: 0.85,
    child: AnimatedContainer(
      duration: const Duration(milliseconds: 450),
      curve: Springs.curve,
      width: 44,
      height: 44,
      decoration: BoxDecoration(color: color, borderRadius: BorderRadius.circular(selected ? 14 : 22)),
      child: AnimatedSwitcher(
        duration: const Duration(milliseconds: 250),
        child: selected ? const Icon(Icons.check_rounded, key: ValueKey('c'), color: Colors.white, size: 24) : const SizedBox.shrink(key: ValueKey('n')),
      ),
    ),
  );
}
