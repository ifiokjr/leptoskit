import 'package:code_assets/code_assets.dart';
import 'package:hooks/hooks.dart';
import 'package:native_toolchain_c/native_toolchain_c.dart';

/// Compiles `src/arena_echo.c`, the sync tier of the `echo` scenario, into a
/// code asset bound by `lib/src/echo/arena_echo_ffi.dart`.
Future<void> main(List<String> arguments) async {
  await build(arguments, (input, output) async {
    if (!input.config.buildCodeAssets) return;
    await CBuilder.library(
      name: 'arena_echo',
      assetName: 'src/echo/arena_echo_ffi.dart',
      sources: ['src/arena_echo.c'],
    ).run(input: input, output: output);
  });
}
