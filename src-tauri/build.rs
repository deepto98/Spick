fn main() {
    let target_is_macos = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos");
    if target_is_macos && std::env::var_os("CARGO_FEATURE_MACOS_INPUT_METHOD_PROTOTYPE").is_some() {
        let mut peer_identity = cc::Build::new();
        peer_identity
            .file("../macos-input-method/Sources/SpickPeerIdentity.m")
            .flag("-fobjc-arc")
            .flag("-fmodules")
            .flag("-mmacosx-version-min=13.0");
        let unsafe_development =
            std::env::var_os("CARGO_FEATURE_MACOS_INPUT_METHOD_UNSAFE_DEV_PEERS").is_some();
        peer_identity.define(
            "SPICK_ALLOW_UNSAFE_ADHOC_PEERS",
            if unsafe_development { "1" } else { "0" },
        );
        peer_identity.compile("spick_peer_identity");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rerun-if-changed=../macos-input-method/Sources/SpickPeerIdentity.h");
        println!("cargo:rerun-if-changed=../macos-input-method/Sources/SpickPeerIdentity.m");
    }
    println!("cargo:rerun-if-env-changed=CARGO_CFG_TARGET_OS");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_MACOS_INPUT_METHOD_PROTOTYPE");
    println!("cargo:rerun-if-env-changed=CARGO_FEATURE_MACOS_INPUT_METHOD_UNSAFE_DEV_PEERS");
    tauri_build::build()
}
