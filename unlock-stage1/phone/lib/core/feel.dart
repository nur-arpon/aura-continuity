// The whole app answers the finger like a watch bezel: every list notches as it scrolls, a page
// swipe clicks as each page lands, and the end of a list is a soft wall. One listener at the app
// root (main.dart) covers every scrollable, so new screens get it for free.
import 'package:flutter/widgets.dart';

import 'fx.dart';

class ScrollFeel extends StatefulWidget {
  const ScrollFeel({super.key, required this.child});
  final Widget child;

  /// Distance between notches on a normal list, in logical pixels (about one row).
  static const double notch = 64;

  @override
  State<ScrollFeel> createState() => _ScrollFeelState();
}

class _ScrollFeelState extends State<ScrollFeel> {
  final _last = <int, double>{};    // scrollable -> pixels at its last notch (or page index)
  final _atEdge = <int, bool>{};
  DateTime _lastBuzz = DateTime.fromMillisecondsSinceEpoch(0);

  // A fast fling must not turn into one long buzz: at most one notch every 30 ms.
  void _notch() {
    final now = DateTime.now();
    if (now.difference(_lastBuzz).inMilliseconds < 30) return;
    _lastBuzz = now;
    Fx.detent();
  }

  bool _on(ScrollNotification n) {
    final id = identityHashCode(n.context);
    final m = n.metrics;
    if (n is ScrollStartNotification) {
      _last[id] = m is PageMetrics ? (m.page ?? 0).roundToDouble() : m.pixels;
    } else if (n is ScrollUpdateNotification) {
      final last = _last[id] ?? m.pixels;
      if (m is PageMetrics) {
        final page = (m.page ?? 0).roundToDouble();
        if (page != last) { _last[id] = page; _notch(); }
      } else if ((m.pixels - last).abs() >= ScrollFeel.notch) {
        _last[id] = m.pixels;
        _notch();
      }
      if (m.pixels > m.minScrollExtent && m.pixels < m.maxScrollExtent) _atEdge[id] = false;
    } else if (n is OverscrollNotification) {
      if (_atEdge[id] != true) { _atEdge[id] = true; Fx.edge(); }
    } else if (n is ScrollEndNotification) {
      _last.remove(id);
    }
    return false;   // never swallow: the screens' own listeners still see everything
  }

  @override
  Widget build(BuildContext context) => NotificationListener<ScrollNotification>(onNotification: _on, child: widget.child);
}
