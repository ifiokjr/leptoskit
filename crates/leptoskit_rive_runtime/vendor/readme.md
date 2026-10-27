# Native runtime sources

`runtime.tar.gz` contains the unmodified `include/`, `src/`, and `LICENSE` from Rive's MIT-licensed C++ runtime, commit `a203f5355840bbc22c4544fef9edc411c85f3c03`: <https://github.com/rive-app/rive-runtime/tree/a203f5355840bbc22c4544fef9edc411c85f3c03>.

The build verifies SHA-256 `5d2106870e8caeb9a69ebd702e3874560ca6f628ec0eaed1b0c1006f59521872` before extracting the archive. Builds perform no downloads or subprocess-driven dependency installation. The source license is included in the archive and in `LICENSE.rive`; retain its notice when redistributing the native runtime.

The native build omits scripting, audio, text shaping, GPU scripting, networking, and layout engines. Files requiring unsupported features must be rejected by the adapter rather than presented as complete renderings.
