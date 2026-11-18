// Aura, phone app (item 13). The Bluetooth / Keystore / Aura Link work lives in Kotlin
// (android/app/src/main/kotlin/...); this is the face: onboarding, the PC at a glance, the big
// Lock PC / Unlock PC buttons, Continue from laptop / on PC, the Handoff inbox and Settings.
import 'package:dynamic_color/dynamic_color.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'core/bridge.dart';
import 'core/feel.dart';
import 'core/fx.dart';
import 'core/model.dart';
import 'core/theme.dart';
import 'core/widgets.dart';
import 'core/motion.dart';
import 'screens/home.dart';
import 'screens/remote.dart';
import 'screens/onboarding.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  SystemChrome.setEnabledSystemUIMode(SystemUiMode.edgeToEdge);
  final model = AuraModel(ChannelBridge());
  model.openRemoteTab = (tab) => auraNav.currentState?.push(auraRoute(RemoteScreen(initialTab: tab)));
  model.start();
  runApp(AuraApp(model: model));
}

final GlobalKey<NavigatorState> auraNav = GlobalKey<NavigatorState>();

class AuraApp extends StatelessWidget {
  const AuraApp({super.key, required this.model, this.themeMode = ThemeMode.system, this.home});
  final AuraModel model;
  final ThemeMode themeMode;

  /// Tests pin a screen here; the app decides by itself otherwise.
  final Widget? home;

  /// Expressive = Material 3 from the wallpaper (Material You) or a seed colour; Classic = the old Aura theme.
  static ThemeData themeFor(AuraModel m, Brightness b, ColorScheme? wall) {
    // the model notifies every poll: only rebuild the (big) ThemeData when the look really changed
    final key = (m.expressive, m.seed, wall);
    final hit = _themes[b];
    if (hit != null && hit.$1 == key) return hit.$2;
    final ThemeData t;
    if (!m.expressive) {
      t = auraTheme(b);
    } else {
      final seed = m.seed;
      final cs = seed != null || wall == null
          ? ColorScheme.fromSeed(seedColor: Color(seed ?? auraDefaultSeed), brightness: b)
          : wall;
      t = expressiveTheme(cs);
    }
    _themes[b] = (key, t);
    return t;
  }

  static final _themes = <Brightness, (Object, ThemeData)>{};

  @override
  Widget build(BuildContext context) => AuraScope(
    model: model,
    child: DynamicColorBuilder(
      builder: (light, dark) => ListenableBuilder(
        listenable: model,
        builder: (context, _) => MaterialApp(
          navigatorKey: auraNav,
          navigatorObservers: [_PageFeel()],
          title: 'Aura',
          debugShowCheckedModeBanner: false,
          theme: themeFor(model, Brightness.light, light),
          darkTheme: themeFor(model, Brightness.dark, dark),
          themeMode: themeMode,
          themeAnimationDuration: const Duration(milliseconds: 450),
          themeAnimationCurve: Springs.smoothCurve,
          builder: (context, child) {
            final dark = Theme.of(context).brightness == Brightness.dark;
            return AnnotatedRegion<SystemUiOverlayStyle>(
              value: (dark ? SystemUiOverlayStyle.light : SystemUiOverlayStyle.dark).copyWith(
                statusBarColor: Colors.transparent,
                systemNavigationBarColor: Colors.transparent,
                systemNavigationBarContrastEnforced: false,
              ),
              child: ToastHost(child: ScrollFeel(child: child!)),
            );
          },
          home: home ?? const _Root(),
        ),
      ),
    ),
  );
}

/// A tick whenever a page opens or closes (the dial / button / toggle ticks live in their widgets).
class _PageFeel extends NavigatorObserver {
  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    if (previousRoute != null && route is! PopupRoute) Fx.tick();
  }

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) {
    if (route is! PopupRoute) Fx.tick();
  }
}

/// Onboarding until paired and finished; Home after that. Never a dead end: forgetting the PC
/// brings the onboarding back, and a half-finished setup shows a "Finish setup" row on Home.
class _Root extends StatefulWidget {
  const _Root();
  @override
  State<_Root> createState() => _RootState();
}

class _RootState extends State<_Root> {
  bool _onboarding = false;

  @override
  Widget build(BuildContext context) {
    final m = AuraScope.of(context);
    if (!m.loaded) {
      return const Scaffold(
        body: AuraBackground(child: Center(child: AuraOrb(size: 120))),
      );
    }
    if (!m.paired) _onboarding = true;
    final Widget page = _onboarding ? Onboarding(onDone: () => setState(() => _onboarding = false)) : const HomeScreen();
    return AnimatedSwitcher(
      duration: const Duration(milliseconds: 500),
      switchInCurve: Curves.easeOutCubic,
      child: KeyedSubtree(key: ValueKey(_onboarding), child: page),
    );
  }
}
