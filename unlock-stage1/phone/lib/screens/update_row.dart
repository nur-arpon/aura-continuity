// In-app updater row: "Update to build N" on Home and in Settings when a verified newer Aura APK
// (delivered by the PC over Aura Link) is waiting. One tap opens the system installer.
import 'package:flutter/material.dart';

import '../core/model.dart';
import '../core/theme.dart';
import '../core/widgets.dart';

class UpdateRow extends StatelessWidget {
  const UpdateRow({super.key, required this.rowKey});
  final Key rowKey;

  @override
  Widget build(BuildContext context) {
    final m = AuraScope.of(context);
    final build = (m.s['updateBuild'] as int?) ?? 0;
    if (build <= 0) return const SizedBox.shrink();
    return Padding(
      padding: const EdgeInsets.only(top: 12, bottom: 4),
      child: Group(
        children: [
          Row2(
            rowKey: rowKey,
            icon: Icons.system_update_rounded,
            tone: Tone.good,
            title: 'Update to build $build',
            subtitle: 'Aura ${(m.s['updateVersion'] as String?) ?? ''} is ready, sent by your PC. Tap to install.',
            trailing: PillButton('Update', cue: null, onTap: m.installUpdate),
            onTap: m.installUpdate,
          ),
        ],
      ),
    );
  }
}
