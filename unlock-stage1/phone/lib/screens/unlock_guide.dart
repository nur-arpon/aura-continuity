// "Keep my phone unlocked near my laptop" (item 15): a guide for Android's Extend Unlock / Smart Lock
// "Trusted devices". Aura only explains and opens the right Settings page; it never changes a
// security setting. The owner turns it on himself.
import 'package:flutter/material.dart';

import '../core/fx.dart';
import '../core/model.dart';
import '../core/motion.dart';
import '../core/theme.dart';
import '../core/widgets.dart';
import 'inbox.dart' show PageHeader;

class UnlockGuideScreen extends StatelessWidget {
  const UnlockGuideScreen({super.key});

  @override
  Widget build(BuildContext context) {
    final m = AuraScope.of(context);
    final a = context.aura;
    final pc = m.pcName;
    var i = 0;
    final steps = <(String, String, VoidCallback?, String?)>[
      ('Pair $pc in Bluetooth', 'Your laptop is already paired for Aura. Check it is in the list of paired devices.', () => m.openSettingsPage('bluetooth'), 'Open Bluetooth'),
      ('Open the Extend Unlock page', 'Settings > Security and privacy > More security settings > Extend Unlock. On some phones it is Lock screen > Secure lock settings > Smart Lock > Trusted devices.', () => m.openSettingsPage('security'), 'Open security settings'),
      ('Choose Trusted devices', 'You need a screen lock (PIN, pattern or fingerprint) for this to appear.', null, null),
      ('Add $pc', 'Tap Add trusted device and pick $pc. Confirm the question.', null, null),
      ('Check it works', 'Lock the phone with $pc connected: it stays open while the laptop is near. Walk away and it locks again.', null, null),
    ];
    return Scaffold(
      body: AuraBackground(
        child: SafeArea(
          bottom: false,
          child: ListView(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 40),
            children: [
              const PageHeader(title: 'Stay unlocked', subtitle: 'Keep your phone unlocked near your laptop'),
              const SizedBox(height: 14),
              Entrance(
                index: i++,
                child: AuraCard(
                  child: Row(
                    children: [
                      const IconTile(Icons.laptop_chromebook_rounded, size: 46),
                      const SizedBox(width: 14),
                      Expanded(
                        child: Text(
                          'Android can keep this phone unlocked while it is connected to a Bluetooth device you trust, like your laptop. You switch it on in Android\'s own settings. Aura never changes security settings for you.',
                          style: TextStyle(color: a.text2, fontSize: 13.8, height: 1.45),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
              const SectionLabel('Five steps'),
              for (var k = 0; k < steps.length; k++)
                Entrance(
                  index: i++,
                  child: Padding(
                    padding: const EdgeInsets.only(bottom: 10),
                    child: _Step(n: k + 1, title: steps[k].$1, text: steps[k].$2, action: steps[k].$4, onAction: steps[k].$3),
                  ),
                ),
              const SectionLabel('Good to know'),
              Entrance(
                index: i++,
                child: Group(
                  children: [
                    Row2(icon: Icons.shield_outlined, tone: Tone.warn, title: 'Anyone holding the phone near the laptop can open it', subtitle: 'Turn it off in the same place if you leave the laptop with other people.'),
                    Row2(icon: Icons.bluetooth_connected_rounded, title: 'The laptop must show Connected, not only Paired', subtitle: 'Windows connects when the phone is near and Bluetooth is on. If it only says Paired, tap it in Windows Bluetooth settings once.'),
                    Row2(icon: Icons.timer_outlined, title: 'It locks again a short while after the laptop goes out of range', subtitle: 'Android decides the delay. Aura is not involved in it.'),
                  ],
                ),
              ),
              const SizedBox(height: 18),
              Entrance(
                index: i++,
                child: WideButton('Open security settings', icon: Icons.settings_rounded, onTap: () => m.openSettingsPage('security')),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _Step extends StatelessWidget {
  const _Step({required this.n, required this.title, required this.text, this.action, this.onAction});
  final int n;
  final String title;
  final String text;
  final String? action;
  final VoidCallback? onAction;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return AuraCard(
      radius: 22,
      padding: const EdgeInsets.fromLTRB(14, 14, 14, 14),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 34,
            height: 34,
            alignment: Alignment.center,
            decoration: BoxDecoration(shape: BoxShape.circle, color: a.primary.withValues(alpha: 0.15)),
            child: Text('$n', style: TextStyle(color: a.primary, fontWeight: FontWeight.w800, fontSize: 16)),
          ),
          const SizedBox(width: 12),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(title, style: TextStyle(color: a.text, fontSize: 15.5, fontWeight: FontWeight.w700)),
                const SizedBox(height: 3),
                Text(text, style: TextStyle(color: a.text3, fontSize: 13, height: 1.4)),
                if (action != null) ...[
                  const SizedBox(height: 10),
                  Align(alignment: Alignment.centerLeft, child: PillButton(action!, cue: Cue.tap, onTap: onAction)),
                ],
              ],
            ),
          ),
        ],
      ),
    );
  }
}
