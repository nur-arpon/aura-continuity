// Aura look: the Continuity design tokens (design/tokens.css) as a Flutter theme. Navy-first, one
// bright blue for action and state, teal = all good, amber = needs you. Same palette as the watch.
import 'package:dynamic_color/dynamic_color.dart';
import 'package:flutter/material.dart';

@immutable
class AuraColors extends ThemeExtension<AuraColors> {
  const AuraColors({
    required this.bg,
    required this.bgTop,
    required this.surface1,
    required this.surface2,
    required this.surface3,
    required this.divider,
    required this.border,
    required this.text,
    required this.text2,
    required this.text3,
    required this.primary,
    required this.primaryBright,
    required this.onPrimary,
    required this.success,
    required this.warning,
    required this.danger,
    required this.sky,
    required this.glow,
    required this.dark,
    this.expressive = false,
  });

  final Color bg, bgTop, surface1, surface2, surface3, divider, border;
  final Color text, text2, text3;
  final Color primary, primaryBright, onPrimary, success, warning, danger, sky, glow;
  final bool dark;

  /// true = the Material 3 Expressive look (colours come from a ColorScheme, shapes are rounder).
  final bool expressive;

  static const darkColors = AuraColors(
    bg: Color(0xFF0A1424),
    bgTop: Color(0xFF10234A),
    surface1: Color(0xFF101E36),
    surface2: Color(0xFF172A49),
    surface3: Color(0xFF1E3459),
    divider: Color(0xFF223A61),
    border: Color(0xFF5A77A6),
    text: Color(0xFFE9F0FB),
    text2: Color(0xFFB4C3DB),
    text3: Color(0xFF8FA3C4),
    primary: Color(0xFF5AA2FF),
    primaryBright: Color(0xFF8EC5FF),
    onPrimary: Color(0xFF06142B),
    success: Color(0xFF4CD3B4),
    warning: Color(0xFFF3B95F),
    danger: Color(0xFFFF8A80),
    sky: Color(0xFF8EC5FF),
    glow: Color(0xFF5AA2FF),
    dark: true,
  );

  static const lightColors = AuraColors(
    bg: Color(0xFFEEF3FA),
    bgTop: Color(0xFFDDE8F8),
    surface1: Color(0xFFFAFCFF),
    surface2: Color(0xFFE6EDF7),
    surface3: Color(0xFFDCE5F2),
    divider: Color(0xFFD3DEED),
    border: Color(0xFF7186A8),
    text: Color(0xFF0B1A33),
    text2: Color(0xFF33466A),
    text3: Color(0xFF53658A),
    primary: Color(0xFF1D5FD0),
    primaryBright: Color(0xFF3B82F6),
    onPrimary: Color(0xFFFAFCFF),
    success: Color(0xFF0A7360),
    warning: Color(0xFF8F5200),
    danger: Color(0xFFB3261E),
    sky: Color(0xFF5AA2FF),
    glow: Color(0xFF3B82F6),
    dark: false,
  );

  Color tone(Tone t) => switch (t) {
    Tone.good => success,
    Tone.info => primary,
    Tone.warn => warning,
    Tone.neutral => text3,
  };

  /// The same token names, filled from a Material 3 [ColorScheme] (Material You / seed colour), so every
  /// Aura widget follows the wallpaper in the Expressive look.
  factory AuraColors.fromScheme(ColorScheme cs) {
    final dark = cs.brightness == Brightness.dark;
    Color mix(Color c) => c.harmonizeWith(cs.primary);
    return AuraColors(
      bg: cs.surface,
      bgTop: cs.surfaceContainerLow,
      surface1: cs.surfaceContainerLow,
      surface2: cs.surfaceContainer,
      surface3: cs.surfaceContainerHigh,
      divider: cs.outlineVariant,
      border: cs.outline,
      text: cs.onSurface,
      text2: cs.onSurfaceVariant,
      text3: Color.alphaBlend(cs.onSurfaceVariant.withValues(alpha: 0.82), cs.surface),
      primary: cs.primary,
      primaryBright: cs.primary,
      onPrimary: cs.onPrimary,
      success: mix(dark ? const Color(0xFF6FDBB0) : const Color(0xFF0A7360)),
      warning: mix(dark ? const Color(0xFFF3B95F) : const Color(0xFF8F5200)),
      danger: cs.error,
      sky: cs.secondary,
      glow: cs.primary,
      dark: dark,
      expressive: true,
    );
  }

  @override
  AuraColors copyWith() => this;

  @override
  AuraColors lerp(ThemeExtension<AuraColors>? other, double t) {
    if (other is! AuraColors) return this;
    Color l(Color a, Color b) => Color.lerp(a, b, t)!;
    return AuraColors(
      bg: l(bg, other.bg), bgTop: l(bgTop, other.bgTop), surface1: l(surface1, other.surface1), surface2: l(surface2, other.surface2),
      surface3: l(surface3, other.surface3), divider: l(divider, other.divider), border: l(border, other.border), text: l(text, other.text),
      text2: l(text2, other.text2), text3: l(text3, other.text3), primary: l(primary, other.primary), primaryBright: l(primaryBright, other.primaryBright),
      onPrimary: l(onPrimary, other.onPrimary), success: l(success, other.success), warning: l(warning, other.warning), danger: l(danger, other.danger),
      sky: l(sky, other.sky), glow: l(glow, other.glow),
      dark: t < 0.5 ? dark : other.dark, expressive: t < 0.5 ? expressive : other.expressive,
    );
  }
}

/// What a status means: teal all good, blue informative, amber needs you.
enum Tone { good, info, warn, neutral }

extension AuraTheme on BuildContext {
  AuraColors get aura => Theme.of(this).extension<AuraColors>() ?? AuraColors.darkColors;
}

ThemeData auraTheme(Brightness b) {
  final c = b == Brightness.dark ? AuraColors.darkColors : AuraColors.lightColors;
  final scheme = ColorScheme(
    brightness: b,
    primary: c.primary,
    onPrimary: c.onPrimary,
    secondary: c.sky,
    onSecondary: c.onPrimary,
    error: c.danger,
    onError: c.onPrimary,
    surface: c.surface1,
    onSurface: c.text,
    surfaceContainerHighest: c.surface2,
    outline: c.border,
    outlineVariant: c.divider,
  );
  final base = ThemeData(useMaterial3: true, colorScheme: scheme, brightness: b, fontFamily: 'Roboto');
  final t = base.textTheme.apply(bodyColor: c.text, displayColor: c.text);
  return base.copyWith(
    scaffoldBackgroundColor: c.bg,
    extensions: [c],
    splashFactory: InkSparkle.splashFactory,
    textTheme: t.copyWith(
      displaySmall: t.displaySmall?.copyWith(fontSize: 34, height: 40 / 34, fontWeight: FontWeight.w700, letterSpacing: -0.6),
      headlineSmall: t.headlineSmall?.copyWith(fontSize: 24, height: 30 / 24, fontWeight: FontWeight.w700, letterSpacing: -0.3),
      titleLarge: t.titleLarge?.copyWith(fontSize: 20, height: 26 / 20, fontWeight: FontWeight.w600, letterSpacing: -0.2),
      titleMedium: t.titleMedium?.copyWith(fontSize: 16, height: 22 / 16, fontWeight: FontWeight.w600),
      bodyLarge: t.bodyLarge?.copyWith(fontSize: 16, height: 22 / 16, color: c.text2),
      bodyMedium: t.bodyMedium?.copyWith(fontSize: 14, height: 20 / 14, color: c.text2),
      bodySmall: t.bodySmall?.copyWith(fontSize: 12.5, height: 17 / 12.5, color: c.text3),
      labelLarge: t.labelLarge?.copyWith(fontSize: 15, fontWeight: FontWeight.w600, letterSpacing: 0.1),
      labelSmall: t.labelSmall?.copyWith(fontSize: 11.5, fontWeight: FontWeight.w600, letterSpacing: 1.1, color: c.text3),
    ),
    dividerTheme: DividerThemeData(color: c.divider, thickness: 1, space: 1),
    dialogTheme: DialogThemeData(
      backgroundColor: c.surface1,
      surfaceTintColor: Colors.transparent,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(28)),
    ),
    bottomSheetTheme: BottomSheetThemeData(
      backgroundColor: c.surface1,
      surfaceTintColor: Colors.transparent,
      showDragHandle: true,
      dragHandleColor: c.border,
      shape: const RoundedRectangleBorder(borderRadius: BorderRadius.vertical(top: Radius.circular(28))),
    ),
    switchTheme: SwitchThemeData(
      thumbColor: WidgetStateProperty.resolveWith((s) => s.contains(WidgetState.selected) ? c.onPrimary : c.text3),
      trackColor: WidgetStateProperty.resolveWith((s) => s.contains(WidgetState.selected) ? c.primary : c.surface3),
      trackOutlineColor: WidgetStateProperty.resolveWith((s) => s.contains(WidgetState.selected) ? c.primary : c.border),
    ),
    progressIndicatorTheme: ProgressIndicatorThemeData(color: c.primary, circularTrackColor: c.surface3),
    pageTransitionsTheme: const PageTransitionsTheme(builders: {TargetPlatform.android: PredictiveBackPageTransitionsBuilder()}),
  );
}

/// Seed colours offered in Settings > Appearance (0 = follow the wallpaper).
const auraSeeds = <int>[0xFF3B82F6, 0xFF6750A4, 0xFFD0417E, 0xFFE4572E, 0xFFE0A100, 0xFF2E9B5F, 0xFF0E9AA7, 0xFF7C5E4A];
const auraDefaultSeed = 0xFF3B6FD8;

/// The Material 3 Expressive theme: [cs] comes from the wallpaper (dynamic_color) or a seed colour.
ThemeData expressiveTheme(ColorScheme cs) {
  final c = AuraColors.fromScheme(cs);
  final base = ThemeData(useMaterial3: true, colorScheme: cs, brightness: cs.brightness, fontFamily: 'Roboto');
  final t = base.textTheme.apply(bodyColor: cs.onSurface, displayColor: cs.onSurface);
  const pill = StadiumBorder();
  return base.copyWith(
    scaffoldBackgroundColor: cs.surface,
    extensions: [c],
    splashFactory: InkSparkle.splashFactory,
    // bolder, tighter type scale (Expressive leans on emphasised weights)
    textTheme: t.copyWith(
      displaySmall: t.displaySmall?.copyWith(fontSize: 40, height: 1.1, fontWeight: FontWeight.w800, letterSpacing: -1.2),
      headlineLarge: t.headlineLarge?.copyWith(fontSize: 34, height: 1.12, fontWeight: FontWeight.w800, letterSpacing: -0.9),
      headlineMedium: t.headlineMedium?.copyWith(fontSize: 28, height: 1.15, fontWeight: FontWeight.w800, letterSpacing: -0.6),
      headlineSmall: t.headlineSmall?.copyWith(fontSize: 24, height: 1.2, fontWeight: FontWeight.w700, letterSpacing: -0.3),
      titleLarge: t.titleLarge?.copyWith(fontSize: 21, fontWeight: FontWeight.w700, letterSpacing: -0.2),
      titleMedium: t.titleMedium?.copyWith(fontSize: 16.5, fontWeight: FontWeight.w700),
      labelLarge: t.labelLarge?.copyWith(fontSize: 15, fontWeight: FontWeight.w700, letterSpacing: 0.1),
      labelMedium: t.labelMedium?.copyWith(fontWeight: FontWeight.w700),
      bodyLarge: t.bodyLarge?.copyWith(color: cs.onSurfaceVariant),
      bodyMedium: t.bodyMedium?.copyWith(color: cs.onSurfaceVariant),
      bodySmall: t.bodySmall?.copyWith(color: cs.onSurfaceVariant),
    ),
    filledButtonTheme: FilledButtonThemeData(style: FilledButton.styleFrom(shape: pill, minimumSize: const Size(64, 56))),
    textButtonTheme: TextButtonThemeData(style: TextButton.styleFrom(shape: pill)),
    outlinedButtonTheme: OutlinedButtonThemeData(style: OutlinedButton.styleFrom(shape: pill, minimumSize: const Size(64, 52))),
    iconButtonTheme: IconButtonThemeData(style: IconButton.styleFrom(minimumSize: const Size(48, 48))),
    cardTheme: CardThemeData(elevation: 0, color: cs.surfaceContainerLow, shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(32))),
    dialogTheme: DialogThemeData(backgroundColor: cs.surfaceContainerHigh, shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(36))),
    bottomSheetTheme: BottomSheetThemeData(
      backgroundColor: cs.surfaceContainerLow,
      showDragHandle: true,
      shape: const RoundedRectangleBorder(borderRadius: BorderRadius.vertical(top: Radius.circular(36))),
    ),
    listTileTheme: ListTileThemeData(shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(22))),
    dividerTheme: DividerThemeData(color: cs.outlineVariant.withValues(alpha: 0.6), thickness: 1, space: 1),
    chipTheme: ChipThemeData(shape: pill, side: BorderSide.none),
    segmentedButtonTheme: SegmentedButtonThemeData(style: SegmentedButton.styleFrom(visualDensity: VisualDensity.comfortable)),
    progressIndicatorTheme: ProgressIndicatorThemeData(color: cs.primary, circularTrackColor: cs.surfaceContainerHighest),
    pageTransitionsTheme: const PageTransitionsTheme(builders: {TargetPlatform.android: PredictiveBackPageTransitionsBuilder()}),
  );
}
