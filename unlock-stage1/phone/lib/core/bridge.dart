// The one door to the native side (Kotlin: MainActivity's "unlock" channel). Everything the app
// shows comes from status / handoff / away / inbox; every action is a method call. Tests use a
// fake implementation, so every screen renders without a phone.
import 'package:flutter/services.dart';

abstract class AuraBridge {
  Future<Map<String, dynamic>> status();
  Future<Map<String, dynamic>> handoff();
  Future<Map<String, dynamic>> away();
  Future<List<Map<String, dynamic>>> inbox();

  /// Any other channel method (approve, awayLock, setMode, fx, ...). Returns the native result.
  Future<dynamic> call(String method, [Map<String, dynamic>? args]);
}

class ChannelBridge implements AuraBridge {
  ChannelBridge([MethodChannel? ch]) : _ch = ch ?? const MethodChannel('unlock');
  final MethodChannel _ch;

  Future<Map<String, dynamic>> _map(String m) async => (await _ch.invokeMapMethod<String, dynamic>(m)) ?? const {};

  @override
  Future<Map<String, dynamic>> status() => _map('status');
  @override
  Future<Map<String, dynamic>> handoff() => _map('handoff');
  @override
  Future<Map<String, dynamic>> away() => _map('away');
  @override
  Future<List<Map<String, dynamic>>> inbox() async {
    final l = await _ch.invokeListMethod<dynamic>('inbox') ?? const [];
    return [for (final e in l) (e as Map).cast<String, dynamic>()];
  }

  @override
  Future<dynamic> call(String method, [Map<String, dynamic>? args]) => _ch.invokeMethod<dynamic>(method, args);
}
