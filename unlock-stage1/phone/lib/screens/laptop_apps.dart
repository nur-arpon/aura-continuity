// "Laptop apps": the PC's apps with their real icons. Tap one to open it on the laptop (the PC brings that app up and
// Artemis streams just it); the pin button puts it on the home screen. The list is kept on the phone, so it shows at once;
// opening the picker asks the PC for a fresh one.
import 'dart:async';
import 'dart:typed_data';

import 'package:flutter/material.dart';

import '../core/model.dart';
import '../core/theme.dart';

class _LApp {
  _LApp(this.id, this.name, this.icon, this.recent);
  final String id;
  final String name;
  final Uint8List? icon;
  final bool recent;
}

Future<void> showLaptopApps(BuildContext context) {
  final m = AuraScope.of(context);
  return showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    builder: (_) => AuraScope(model: m, child: const _LaptopAppsSheet()),
  );
}

class _LaptopAppsSheet extends StatefulWidget {
  const _LaptopAppsSheet();
  @override
  State<_LaptopAppsSheet> createState() => _LaptopAppsSheetState();
}

class _LaptopAppsSheetState extends State<_LaptopAppsSheet> {
  List<_LApp> apps = const [];
  Timer? _poll;
  int _polls = 0;
  String _q = '';
  AuraModel? _m;

  Future<void> _load() async {
    final m = _m;
    if (m == null) return;
    final raw = await m.bridge.call('laptopApps').catchError((_) => null);
    if (raw is! List || !mounted) return;
    final list = <_LApp>[
      for (final e in raw)
        if (e is Map) _LApp('${e['id']}', '${e['name']}', e['icon'] is Uint8List ? e['icon'] as Uint8List : null, e['recent'] == true),
    ];
    if (list.length != apps.length || list.any((a) => !apps.any((b) => b.id == a.id))) setState(() => apps = list);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_m != null) return;
    _m = AuraScope.of(context);
    _load();
    _m!.bridge.call('laptopAppsAsk').catchError((_) => null);
    // the PC answers within a few seconds, in several parts: look a few times, then stop (nothing polls in the background)
    _poll = Timer.periodic(const Duration(milliseconds: 900), (t) {
      _load();
      if (++_polls >= 8) t.cancel();
    });
  }

  @override
  void dispose() {
    _poll?.cancel();
    super.dispose();
  }

  Future<void> _pin(_LApp a) async {
    final m = _m!;
    final ok = await m.bridge.call('pinLaptopApp', {'id': a.id}).catchError((_) => false) == true;
    m.show(AuraToast(ok ? 'Confirm on your home screen' : 'This launcher cannot pin shortcuts', tone: ok ? Tone.good : Tone.info, icon: Icons.add_to_home_screen_rounded));
  }

  @override
  Widget build(BuildContext context) {
    final a = context.aura;
    final q = _q.trim().toLowerCase();
    final shown = [for (final x in apps) if (q.isEmpty || x.name.toLowerCase().contains(q)) x];
    return SafeArea(
      child: SizedBox(
        height: MediaQuery.of(context).size.height * 0.78,
        child: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(20, 0, 20, 8),
              child: Row(children: [
                Expanded(child: Text('Laptop apps', style: TextStyle(color: a.text, fontSize: 20, fontWeight: FontWeight.w700))),
                if (apps.isEmpty) const SizedBox(width: 18, height: 18, child: CircularProgressIndicator(strokeWidth: 2)),
              ]),
            ),
            Padding(
              padding: const EdgeInsets.fromLTRB(20, 0, 20, 8),
              child: Text('Tap to open on the laptop. The pin adds it to your home screen.', style: TextStyle(color: a.text3)),
            ),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 16),
              child: TextField(
                key: const ValueKey('laptop-apps-search'),
                onChanged: (v) => setState(() => _q = v),
                decoration: const InputDecoration(prefixIcon: Icon(Icons.search_rounded), hintText: 'Search apps', isDense: true),
              ),
            ),
            Expanded(
              child: apps.isEmpty
                  ? Center(child: Text('Asking your laptop...', style: TextStyle(color: a.text3)))
                  : ListView.builder(
                      itemCount: shown.length,
                      itemBuilder: (_, i) {
                        final x = shown[i];
                        return ListTile(
                          key: ValueKey('lapp-${x.id}'),
                          leading: SizedBox(
                            width: 40,
                            height: 40,
                            child: x.icon != null
                                ? Image.memory(x.icon!, filterQuality: FilterQuality.medium, gaplessPlayback: true)
                                : CircleAvatar(child: Text(x.name.isEmpty ? '?' : x.name[0].toUpperCase())),
                          ),
                          title: Text(x.name),
                          subtitle: x.recent ? const Text('Open on the laptop now') : null,
                          trailing: IconButton(
                            key: ValueKey('lapp-pin-${x.id}'),
                            tooltip: 'Add to home screen',
                            icon: const Icon(Icons.add_to_home_screen_rounded),
                            onPressed: () => _pin(x),
                          ),
                          onTap: () => _m!.bridge.call('openLaptopApp', {'id': x.id, 'name': x.name}).catchError((_) => null),
                        );
                      },
                    ),
            ),
          ],
        ),
      ),
    );
  }
}
