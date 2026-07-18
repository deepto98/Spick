// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(target_os = "macos", feature = "macos-input-method-prototype"))]
extern "C" {
    fn SpickPeerAuthenticationAllowsUnsafeDevelopment() -> bool;
}

fn main() {
    if std::env::args_os().nth(1).as_deref()
        == Some(std::ffi::OsStr::new("--print-input-method-peer-auth-mode"))
    {
        #[cfg(all(target_os = "macos", feature = "macos-input-method-prototype"))]
        {
            let unsafe_development = unsafe { SpickPeerAuthenticationAllowsUnsafeDevelopment() };
            println!(
                "{}",
                if unsafe_development {
                    "unsafe-adhoc"
                } else {
                    "secure"
                }
            );
            return;
        }
        #[cfg(not(all(target_os = "macos", feature = "macos-input-method-prototype")))]
        {
            eprintln!("input-method peer authentication is not compiled into this build");
            std::process::exit(2);
        }
    }
    spick_desktop_lib::run()
}
