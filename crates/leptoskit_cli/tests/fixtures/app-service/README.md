# Generated native application service fixture

This application owns a real channel-driven native thread and its joined close future. The CLI test writes the actual generated registrant to `LEPTOSKIT_SERVICE_FIXTURE_OUTPUT`; build this fixture with `SERVICE_REGISTRANT_DIR` naming that directory. Use the same explicit Cargo/rustc toolchain as the parent test and a separate target directory. The compiler includes the generated registrant directly.

The executable checks constructor failure before task creation, inert registration, activation, real task completion and wake, Pending close, and native join before close succeeds. It runs on the native host; it does not establish installed desktop, Android or iOS execution. Its service intentionally has no Solana dependency. A Solana integration belongs to application-owned client code; on-chain programs use Pina or official Solana program libraries.
