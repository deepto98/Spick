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

    #[cfg(not(all(
        target_os = "macos",
        feature = "macos-input-method-compatibility-harness"
    )))]
    if std::env::args_os().skip(1).any(|argument| {
        argument
            .to_str()
            .is_some_and(|argument| argument.contains("input-method-compatibility"))
    }) {
        eprintln!("input-method compatibility commands are not compiled into this build");
        std::process::exit(2);
    }

    #[cfg(all(
        target_os = "macos",
        feature = "macos-input-method-compatibility-harness"
    ))]
    match spick_desktop_lib::compatibility::prepare_process() {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(64);
        }
    }

    spick_desktop_lib::run()
}
