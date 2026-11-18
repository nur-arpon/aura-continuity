// Haptics + sounds. The patterns, intensity, sounds and the silent / Do Not Disturb rules live in
// Kotlin (Feedback.kt) so the app, the widget and the tiles feel exactly the same. Each action
// has its own cue; nothing here waits for the result.
import 'bridge.dart';

enum Cue { tick, tap, toggleOn, toggleOff, lock, unlock, send, arrive, success, approved, error, slide, detent, edge }

abstract final class Fx {
  static AuraBridge? bridge;

  /// Every cue played (tests read this).
  static final List<Cue> played = [];

  static String _name(Cue c) => switch (c) {
    Cue.toggleOn => 'toggle_on',
    Cue.toggleOff => 'toggle_off',
    _ => c.name,
  };

  static void play(Cue c) {
    played.add(c);
    if (played.length > 200) played.removeAt(0);
    bridge?.call('fx', {'cue': _name(c)}).catchError((_) => null);
  }

  static void tick() => play(Cue.tick);
  static void tap() => play(Cue.tap);
  static void toggle(bool on) => play(on ? Cue.toggleOn : Cue.toggleOff);
  static void error() => play(Cue.error);
  static void success() => play(Cue.success);

  /// Felt, never heard: one watch-bezel notch (dial steps, scrolling) and the soft wall at an end.
  static void detent() => play(Cue.detent);
  static void edge() => play(Cue.edge);
}
