// Every test renders with the real fonts (Roboto + Material icons from the Flutter SDK), so text
// widths and overflows are what the phone shows, not the square test font.
import 'dart:async';
import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

const _fonts = 'D:/flutter/bin/cache/artifacts/material_fonts';

Future<void> testExecutable(FutureOr<void> Function() testMain) async {
  TestWidgetsFlutterBinding.ensureInitialized();
  Future<ByteData> bytes(String f) async => ByteData.sublistView(await File(f).readAsBytes());
  final roboto = FontLoader('Roboto');
  for (final f in ['regular', 'medium', 'bold', 'black', 'light', 'italic']) {
    roboto.addFont(bytes('$_fonts/roboto-$f.ttf'));
  }
  await roboto.load();
  await (FontLoader('MaterialIcons')..addFont(bytes('$_fonts/materialicons-regular.otf'))).load();
  // Android's 'monospace' is Droid Sans Mono; Windows' Consolas is the closest stand-in for the
  // review screenshots (used only to render tests on this PC, never shipped).
  const mono = 'C:/Windows/Fonts/consola.ttf';
  if (File(mono).existsSync()) {
    await (FontLoader('monospace')
          ..addFont(bytes(mono))
          ..addFont(bytes('C:/Windows/Fonts/consolab.ttf')))
        .load();
  }
  await testMain();
}
