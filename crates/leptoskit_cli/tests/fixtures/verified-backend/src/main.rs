fn main() {
    if let Some(path) = std::env::var_os("LEPTOSKIT_RPC_SPEC_OUT") {
        std::fs::write(path, r#"{"leptoskit_rpc_spec":3,"title":"fixture","version":"0.1.0","functions":[]}"#).unwrap();
        return;
    }
    panic!("database/auth startup must not be reached during export");
}
