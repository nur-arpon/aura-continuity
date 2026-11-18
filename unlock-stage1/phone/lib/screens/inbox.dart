// Handoff inbox: everything that went between the phone and the PC (links, videos at their
// second, files, clipboard), newest first, grouped by day. Tap to open again, swipe to remove.
import 'package:flutter/material.dart';

import '../core/fx.dart';
import '../core/model.dart';
import '../core/motion.dart';
import '../core/status.dart';
import '../core/theme.dart';
import '../core/widgets.dart';

IconData kindIcon(String kind) => switch (kind) {
  'video' => Icons.smart_display_rounded,
  'file' => Icons.insert_drive_file_rounded,
  'clip' => Icons.content_paste_rounded,
  'text' => Icons.notes_rounded,
  _ => Icons.public_rounded,
};

String dayOf(int at, int now) {
  final d = DateTime.fromMillisecondsSinceEpoch(at);
  final n = DateTime.fromMillisecondsSinceEpoch(now);
  final a = DateTime(d.year, d.month, d.day);
  final b = DateTime(n.year, n.month, n.day);
  final diff = b.difference(a).inDays;
  if (diff <= 0) return 'Today';
  if (diff == 1) return 'Yesterday';
  if (diff < 7) return const ['Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday', 'Sunday'][d.weekday - 1];
  return '${d.day} ${const ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][d.month - 1]}';
}

class InboxRow extends StatelessWidget {
  const InboxRow({super.key, required this.item});
  final Map<String, dynamic> item;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final m = AuraScope.read(context);
    final kind = (item['kind'] as String?) ?? 'link';
    final out = item['dir'] == 'out';
    final yt = kind == 'video';
    return Pressable(
      key: ValueKey('inbox-${item['id']}'),
      onTap: () => m.openItem(item),
      cue: null,
      scale: 0.98,
      semanticLabel: '${item['title']}. ${item['sub']}',
      child: Padding(
        padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
        child: Row(
          children: [
            SizedBox(
              width: 42,
              height: 42,
              child: Stack(
                clipBehavior: Clip.none,
                children: [
                  Container(
                    decoration: BoxDecoration(
                      borderRadius: BorderRadius.circular(13),
                      color: yt ? const Color(0xFFE53935).withValues(alpha: a.dark ? 0.22 : 0.12) : a.primary.withValues(alpha: a.dark ? 0.16 : 0.10),
                    ),
                    child: Center(child: Icon(kindIcon(kind), size: 22, color: yt ? const Color(0xFFFF6B6B) : a.primary)),
                  ),
                  Positioned(
                    right: -4,
                    bottom: -4,
                    child: Container(
                      width: 20,
                      height: 20,
                      decoration: BoxDecoration(
                        shape: BoxShape.circle,
                        color: out ? a.primary : a.success,
                        border: Border.all(color: a.surface1, width: 2),
                      ),
                      child: Icon(out ? Icons.north_east_rounded : Icons.south_west_rounded, size: 11, color: a.dark ? a.onPrimary : Colors.white),
                    ),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 14),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    (item['title'] as String?) ?? '',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(color: a.text, fontSize: 15, fontWeight: FontWeight.w600),
                  ),
                  const SizedBox(height: 2),
                  Text(
                    (item['sub'] as String?) ?? '',
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(color: a.text3, fontSize: 12.8),
                  ),
                ],
              ),
            ),
            const SizedBox(width: 8),
            Text(ago((item['at'] as int?) ?? 0, m.now).replaceAll(' ago', ''), style: TextStyle(color: a.text3, fontSize: 12)),
          ],
        ),
      ),
    );
  }
}

/// The last few items on Home.
class InboxPreview extends StatelessWidget {
  const InboxPreview({super.key, required this.items});
  final List<Map<String, dynamic>> items;
  @override
  Widget build(BuildContext context) {
    if (items.isEmpty) return const _Empty(compact: true);
    return Group(children: [for (final it in items) InboxRow(item: it)]);
  }
}

class _Empty extends StatelessWidget {
  const _Empty({this.compact = false});
  final bool compact;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return AuraCard(
      padding: EdgeInsets.symmetric(horizontal: 22, vertical: compact ? 20 : 34),
      child: Column(
        children: [
          if (!compact) ...[const AuraOrb(size: 120, icon: Icons.inbox_rounded), const SizedBox(height: 10)],
          if (compact) Icon(Icons.swap_horiz_rounded, color: a.primary, size: 28),
          if (compact) const SizedBox(height: 8),
          Text(
            'Nothing handed off yet',
            style: TextStyle(color: a.text, fontSize: 16, fontWeight: FontWeight.w700),
          ),
          const SizedBox(height: 6),
          Text(
            'Press Ctrl+Alt+H on your PC to send the tab or video you are on. Share anything here to "Continue on PC" to send it back.',
            textAlign: TextAlign.center,
            style: TextStyle(color: a.text3, fontSize: 13, height: 1.4),
          ),
        ],
      ),
    );
  }
}

class InboxScreen extends StatefulWidget {
  const InboxScreen({super.key});
  @override
  State<InboxScreen> createState() => _InboxScreenState();
}

class _InboxScreenState extends State<InboxScreen> {
  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) AuraScope.read(context).inboxSeen();
    });
  }

  Future<void> _clear(AuraModel m) async {
    Fx.tap();
    final a = context.aura;
    final ok = await showDialog<bool>(
      context: context,
      builder: (c) => AlertDialog(
        title: const Text('Clear Handoff?'),
        content: const Text('This only clears the list. Files stay in Download/Aura and nothing changes on your PC.'),
        actions: [
          TextButton(onPressed: () { Fx.tap(); Navigator.pop(c, false); }, child: const Text('Cancel')),
          TextButton(
            onPressed: () { Fx.tap(); Navigator.pop(c, true); },
            child: Text(
              'Clear',
              style: TextStyle(color: a.danger, fontWeight: FontWeight.w700),
            ),
          ),
        ],
      ),
    );
    if (ok == true) await m.clearInbox();
  }

  @override
  Widget build(BuildContext context) {
    final m = AuraScope.of(context);
    final a = context.aura;
    final groups = <String, List<Map<String, dynamic>>>{};
    for (final it in m.inbox) {
      groups.putIfAbsent(dayOf((it['at'] as int?) ?? 0, m.now), () => []).add(it);
    }
    var i = 0;
    return Scaffold(
      body: AuraBackground(
        child: SafeArea(
          bottom: false,
          child: ListView(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 40),
            children: [
              PageHeader(
                title: 'Handoff',
                subtitle: 'Between this phone and ${m.pcName == 'Your PC' ? 'your PC' : m.pcName}',
                action: m.inbox.isEmpty ? null : PillButton('Clear', key: const ValueKey('clear-inbox'), cue: null, onTap: () => _clear(m)),
              ),
              if (m.inbox.isEmpty) const Entrance(child: _Empty()),
              for (final e in groups.entries) ...[
                SectionLabel(e.key),
                Entrance(
                  index: i++,
                  child: Group(
                    children: [
                      for (final it in e.value)
                        Dismissible(
                          key: ValueKey('d-${it['id']}'),
                          direction: DismissDirection.endToStart,
                          onDismissed: (_) => m.removeItem(it),
                          background: Container(
                            alignment: Alignment.centerRight,
                            padding: const EdgeInsets.only(right: 22),
                            color: a.danger.withValues(alpha: 0.14),
                            child: Icon(Icons.delete_outline_rounded, color: a.danger),
                          ),
                          child: InboxRow(item: it),
                        ),
                    ],
                  ),
                ),
              ],
              if (m.inbox.isNotEmpty)
                Padding(
                  padding: const EdgeInsets.only(top: 18),
                  child: Center(
                    child: Text('Swipe left to remove · files are in Download/Aura', style: TextStyle(color: a.text3, fontSize: 12)),
                  ),
                ),
            ],
          ),
        ),
      ),
    );
  }
}

/// Back button + big title, used by every pushed page.
class PageHeader extends StatelessWidget {
  const PageHeader({super.key, required this.title, this.subtitle, this.action});
  final String title;
  final String? subtitle;
  final Widget? action;
  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    return Padding(
      padding: const EdgeInsets.fromLTRB(0, 6, 0, 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Pressable(
                key: const ValueKey('back'),
                onTap: () => Navigator.of(context).maybePop(),
                semanticLabel: 'Back',
                scale: 0.9,
                child: Container(
                  width: 44,
                  height: 44,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: a.expressive ? a.surface3 : a.surface1.withValues(alpha: a.dark ? 0.7 : 0.9),
                    border: a.expressive ? null : Border.all(color: a.divider),
                  ),
                  child: Icon(Icons.arrow_back_rounded, color: a.expressive ? a.text : a.text2, size: 22),
                ),
              ),
              const Spacer(),
              ?action,
            ],
          ),
          const SizedBox(height: 14),
          Padding(
            padding: const EdgeInsets.only(left: 4),
            child: Text(
              title,
              style: TextStyle(color: a.text, fontSize: a.expressive ? 40 : 32, fontWeight: FontWeight.w800, letterSpacing: a.expressive ? -1.4 : -0.9),
            ),
          ),
          if (subtitle != null)
            Padding(
              padding: const EdgeInsets.only(left: 4, top: 2),
              child: Text(subtitle!, style: TextStyle(color: a.text3, fontSize: 14)),
            ),
        ],
      ),
    );
  }
}
