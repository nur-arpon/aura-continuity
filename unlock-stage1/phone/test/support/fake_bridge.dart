// A stand-in for the Kotlin side: canned status maps + a record of every call.
import 'package:unlock_stage1/core/bridge.dart';

/// 2026-10-21 09:30 local: every golden and test uses this clock.
final fixedNow = DateTime(2026, 10, 21, 9, 30).millisecondsSinceEpoch;

class FakeBridge implements AuraBridge {
  FakeBridge({Map<String, dynamic>? s, Map<String, dynamic>? h, Map<String, dynamic>? a, List<Map<String, dynamic>>? inbox})
    : s = s ?? pairedNearby(),
      h = h ?? handoffOn(),
      a = a ?? awayReady(),
      items = inbox ?? sampleInbox();

  Map<String, dynamic> s;
  Map<String, dynamic> h;
  Map<String, dynamic> a;
  List<Map<String, dynamic>> items;
  final calls = <String>[];
  final args = <String, Map<String, dynamic>?>{};

  /// What a method returns (default: true).
  final results = <String, dynamic>{
    'awayLock': 'Lock sent to your PC',
    'awayContinue': 'Opening Slugterra S2E7',
    'awayRefresh': 'ok',
    'sendClipboard': 'Copied to the PC clipboard',
    'linkCompanion': 'linked',
    'inboxOpen': 'ok',
    'ringPc': 'Ringing your laptop',
    'openLaptop': 'Opening your laptop in Moonlight',
    'mediaCmd': 'ok',
    'present': 'ok',
    'openSettings': 'ok',
    'remoteState': nowPlaying(),
  };

  @override
  Future<Map<String, dynamic>> status() async => s;
  @override
  Future<Map<String, dynamic>> handoff() async => h;
  @override
  Future<Map<String, dynamic>> away() async => a;
  @override
  Future<List<Map<String, dynamic>>> inbox() async => items;

  /// item 25 setup walk: which permissions are granted, which the user will decline, the watch state.
  final setupOrder = ['notifications', 'bluetooth', 'location', 'usage', 'listener', 'accessibility', 'battery', 'overlay', 'bubbles'];
  final setupGranted = <String, bool>{};
  final setupDeny = <String>{};
  final setupAsked = <String>[];
  String watchState = 'connected';

  @override
  Future<dynamic> call(String method, [Map<String, dynamic>? a]) async {
    calls.add(method);
    args[method] = a;
    if (method == 'setupStatus') return [for (final id in setupOrder) {'id': id, 'granted': setupGranted[id] == true}];
    if (method == 'setupRequest') {
      final id = a?['id'] as String;
      setupAsked.add(id);
      if (!setupDeny.contains(id)) setupGranted[id] = true;
      return 'back';
    }
    if (method == 'watchSetup') return {'state': watchState, 'name': 'Galaxy Watch'};
    if (method == 'pair') {
      final ok = (a?['code'] as String? ?? '').replaceAll(' ', '').length == 34 && !(a?['code'] as String).startsWith('BAD');
      if (ok) s = pairedNearby()..['companion'] = false;
      return ok;
    }
    if (method == 'unpair') s = unpaired();
    if (method == 'setHaptics') s = {...s, 'hapticLevel': a?['level']};
    if (method == 'setUi') s = {...s, 'uiLook': ?a?['look'], 'uiSeed': ?a?['seed']};
    if (method == 'setSound') s = {...s, 'sound': a?['on']};
    if (method == 'setMode') s = {...s, 'mode': a?['mode']};
    return results.containsKey(method) ? results[method] : true;
  }

  int count(String m) => calls.where((c) => c == m).length;
}

Map<String, dynamic> unpaired() => {
  'paired': false,
  'missingPerms': ['android.permission.BLUETOOTH_CONNECT', 'android.permission.POST_NOTIFICATIONS'],
  'companionSupported': true,
  'companion': false,
  'log': <String>[],
  'version': '1.0.0 (7)',
};

Map<String, dynamic> pairedNearby() => {
  'paired': true,
  'running': true,
  'auto': false,
  'mode': 'auto',
  'enabled': true,
  'levels': 'tap,auto,fingerprint',
  'advertising': false,
  'gatt': true,
  'rfcomm': true,
  'usb': '',
  'answered': 214,
  'restarts': 0,
  'batteryUnrestricted': true,
  'companion': true,
  'companionSupported': true,
  'pcAddr': '02:00:00:AA:BB:CD',
  'watchPinnedFp': '48B1-85F0-F7AC',
  'watchPendingFp': '',
  'missingPerms': <String>[],
  'pending': '',
  'pendingLevel': -1,
  'log': ['09:29:58 Aura Link to PC UP', '09:29:58 listener started by PC presence: BT connected', '09:12:04 approval mode auto'],
  'pcName': 'MYLAPTOP',
  'pcNear': true,
  'pcLocked': 0,
  'pcLockedAt': fixedNow - 40 * 60000,
  'lastSeen': fixedNow,
  'linkVia': 'rfcomm',
  'unlockBusy': false,
  'inboxUnseen': 2,
  'hapticLevel': 3,
  'sound': true,
  'version': '1.0.0 (7)',
};

Map<String, dynamic> lockedAway() => {
  ...pairedNearby(),
  'pcNear': false,
  'linkVia': 'tailnet 100.101.102.103',
  'pcLocked': 1,
  'pcLockedAt': fixedNow - 12 * 60000,
  'inboxUnseen': 0,
};

Map<String, dynamic> handoffOn() => {
  'clipSync': true,
  'suggest': true,
  'mediaAccess': true,
  'chatHead': true,
  'chatHeadAllowed': true,
  'bubble': true,
  'bubbleSupported': true,
  'bubbleAllowed': true,
  'linkApps': [
    {'key': 'youtube', 'label': 'YouTube', 'mode': 'app'},
    {'key': 'linkedin', 'label': 'LinkedIn', 'mode': 'browser'},
  ],
};

Map<String, dynamic> awayReady() => {
  'code': 'ready',
  'title': 'Away mode: ready',
  'text': 'Your PC (100.101.102.103) is reachable from anywhere. End-to-end encrypted by Aura Link.',
  'action': 'none',
  'pcIp': '100.64.100.9',
  'lastTitle': 'Slugterra S2E7 - The Return of the Shane Gang',
  'lastSub': 'YouTube · 14:32',
  'lastApp': 'youtube',
  'pcLocked': false,
  'result': '',
  'resultOk': false,
  'resultAt': 0,
};

Map<String, dynamic> awayMissing() => {
  'code': 'off',
  'title': 'Tailscale off - tap to turn on',
  'text': 'Away mode works while Tailscale is connected. At home Bluetooth is used anyway.',
  'action': 'open',
  'lastTitle': '',
  'lastSub': '',
  'lastApp': '',
  'pcLocked': false,
  'result': '',
  'resultAt': 0,
};

List<Map<String, dynamic>> sampleInbox() => [
  {
    'id': 6,
    'kind': 'video',
    'dir': 'in',
    'title': 'YouTube video',
    'sub': 'From your PC · 14:32',
    'url': 'https://youtu.be/x',
    'uri': '',
    'mime': '',
    'at': fixedNow - 3 * 60000,
  },
  {
    'id': 5,
    'kind': 'file',
    'dir': 'in',
    'title': 'Screenshot 2026-10-21 091204.png',
    'sub': 'From your PC · 812 KB · Download/Aura',
    'url': '',
    'uri': 'content://x',
    'mime': 'image/png',
    'at': fixedNow - 18 * 60000,
  },
  {
    'id': 4,
    'kind': 'link',
    'dir': 'out',
    'title': 'docs.flutter.dev',
    'sub': 'Opened on your PC',
    'url': 'https://docs.flutter.dev/',
    'uri': '',
    'mime': '',
    'at': fixedNow - 52 * 60000,
  },
  {
    'id': 3,
    'kind': 'clip',
    'dir': 'in',
    'title': 'Clipboard from your PC',
    'sub': 'Text · 48 characters · ready to paste',
    'url': '',
    'uri': '',
    'mime': '',
    'at': fixedNow - 26 * 3600000,
  },
  {
    'id': 2,
    'kind': 'file',
    'dir': 'out',
    'title': 'MCD4490 Assignment 2.pdf',
    'sub': 'Sent to your PC · 4.2 MB · Downloads/Aura',
    'url': '',
    'uri': '',
    'mime': '',
    'at': fixedNow - 27 * 3600000,
  },
];

Map<String, dynamic> nowPlaying({bool playing = true, String title = 'Kannil Kannil', String artist = 'Kunal Ganjawala'}) => {
  'title': title,
  'artist': artist,
  'playing': playing,
  'pos': 153390,
  'dur': 249000,
  'volume': 62,
  'muted': false,
  'art': null,
  'at': fixedNow,
  'laptopRinging': false,
  'phoneRinging': false,
  'presenterKeys': false,
  'allowRing': true,
};
