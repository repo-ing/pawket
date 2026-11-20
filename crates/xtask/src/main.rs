//! Rust-only project build tasks. Run `cargo run -p xtask -- web`.

use std::{
    env,
    error::Error,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::{self, Command},
};

const WASM_BINDGEN_VERSION: &str = "0.2.130";
type Result<T> = std::result::Result<T, Box<dyn Error>>;

fn main() {
    let mut arguments = env::args().skip(1);
    let task = arguments.next();
    if arguments.next().is_some() {
        eprintln!("Usage: cargo run -p xtask -- web");
        process::exit(2);
    }
    let result = match task.as_deref() {
        Some("web") => build_web(),
        Some("--help" | "-h" | "help") | None => {
            println!("Pawket build tasks\n\n  cargo run -p xtask -- web\n\nBuild the Rust/WASM browser client and copy its packaged files into the Chrome extension.");
            Ok(())
        }
        Some(unknown) => {
            Err(format!("Unknown task: {unknown}. Use: cargo run -p xtask -- web").into())
        }
    };
    if let Err(error) = result {
        eprintln!("Pawket build failed: {error}");
        process::exit(1);
    }
}

fn build_web() -> Result<()> {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("Cannot locate the Pawket workspace")?;
    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let wasm_bindgen = find_wasm_bindgen(&cargo);
    check_wasm_bindgen(&wasm_bindgen)?;

    let target_directory = match env::var_os("CARGO_TARGET_DIR") {
        Some(directory) if Path::new(&directory).is_absolute() => PathBuf::from(directory),
        Some(directory) => workspace.join(directory),
        None => workspace.join("target"),
    };
    println!("Building Rust browser client for wasm32-unknown-unknown…");
    let status = Command::new(&cargo)
        .current_dir(workspace)
        .args([
            "build",
            "--locked",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            "pawket-browser",
            "--target-dir",
        ])
        .arg(&target_directory)
        .status()?;
    if !status.success() {
        return Err("Rust/WASM compilation failed. If the target is missing, run: rustup target add wasm32-unknown-unknown".into());
    }

    let wasm = target_directory.join("wasm32-unknown-unknown/release/pawket_browser.wasm");
    if !wasm.is_file() {
        return Err(format!("The browser build did not produce {}", wasm.display()).into());
    }
    let web_package = workspace.join("web/pkg");
    println!("Generating the browser's JavaScript/WASM bindings…");
    let status = Command::new(&wasm_bindgen)
        .current_dir(workspace)
        .args(["--target", "web", "--out-dir"])
        .arg(&web_package)
        .arg(&wasm)
        .status()?;
    if !status.success() {
        return Err("wasm-bindgen failed to generate the browser package".into());
    }
    for filename in ["pawket_browser.js", "pawket_browser_bg.wasm"] {
        if !web_package.join(filename).is_file() {
            return Err(format!("Missing expected generated browser file: {filename}").into());
        }
    }

    let extension_package = workspace.join("extension/pkg");
    let copied = copy_package(&web_package, &extension_package)?;
    println!("Ready: web/pkg and extension/pkg ({copied} generated files copied).");
    println!("The web UI and Chrome extension now use the same Rust browser build.");
    Ok(())
}

fn find_wasm_bindgen(cargo: &OsString) -> OsString {
    // Cargo normally exports its full executable path when invoking an xtask.
    // A sibling CLI works even when ~/.cargo/bin was not added to PATH.
    let executable = if cfg!(windows) {
        "wasm-bindgen.exe"
    } else {
        "wasm-bindgen"
    };
    if let Some(directory) = Path::new(cargo)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        let candidate = directory.join(executable);
        if candidate.is_file() {
            return candidate.into_os_string();
        }
    }
    OsString::from(executable)
}

fn check_wasm_bindgen(executable: &OsString) -> Result<()> {
    let installation =
        format!("cargo install --locked wasm-bindgen-cli --version {WASM_BINDGEN_VERSION}");
    let output = Command::new(executable).arg("--version").output()
        .map_err(|error| format!("wasm-bindgen CLI is unavailable ({error}). Install the version matching Cargo.lock:\n  {installation}"))?;
    if !output.status.success() {
        return Err(format!(
            "Could not run wasm-bindgen --version. Reinstall it with:\n  {installation}"
        )
        .into());
    }
    let version_text = String::from_utf8_lossy(&output.stdout);
    let version = version_text.split_whitespace().nth(1).unwrap_or("unknown");
    if version != WASM_BINDGEN_VERSION {
        return Err(format!("wasm-bindgen CLI {version} does not match this project's {WASM_BINDGEN_VERSION}. Install the matching version:\n  {installation}").into());
    }
    Ok(())
}

/// Copy only regular files/directories. Refuse symlinks instead of following a
/// generated package outside the workspace or overwriting a linked destination.
fn copy_package(source: &Path, destination: &Path) -> io::Result<usize> {
    ensure_real_directory(source, false)?;
    ensure_real_directory(destination, true)?;
    let mut entries = fs::read_dir(source)?.collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut copied = 0;
    for entry in entries {
        let target = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Generated browser packages must not contain symlinks",
            ));
        }
        if kind.is_dir() {
            copied += copy_package(&entry.path(), &target)?;
        } else if kind.is_file() {
            if let Ok(metadata) = fs::symlink_metadata(&target) {
                if !metadata.file_type().is_file() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "A generated package destination is not a regular file",
                    ));
                }
            }
            fs::copy(entry.path(), target)?;
            copied += 1;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Generated browser packages may contain only regular files and directories",
            ));
        }
    }
    Ok(copied)
}

fn ensure_real_directory(path: &Path, create: bool) -> io::Result<()> {
    if create {
        fs::create_dir_all(path)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_dir() {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Generated package path must be a real directory",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);
    impl TestDirectory {
        fn new() -> Self {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "pawket-xtask-test-{}-{timestamp}-{id}",
                process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn copies_nested_generated_files_and_replaces_old_bytes() {
        let temp = TestDirectory::new();
        let source = temp.0.join("source");
        let destination = temp.0.join("destination");
        fs::create_dir_all(source.join("snippets/module")).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("pawket_browser.js"), b"new binding").unwrap();
        fs::write(source.join("pawket_browser_bg.wasm"), [0, 97, 115, 109]).unwrap();
        fs::write(source.join("snippets/module/bridge.js"), b"bridge").unwrap();
        fs::write(destination.join("pawket_browser.js"), b"old binding").unwrap();
        assert_eq!(copy_package(&source, &destination).unwrap(), 3);
        assert_eq!(
            fs::read(destination.join("pawket_browser.js")).unwrap(),
            b"new binding"
        );
        assert_eq!(
            fs::read(destination.join("snippets/module/bridge.js")).unwrap(),
            b"bridge"
        );
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_in_the_generated_package() {
        let temp = TestDirectory::new();
        let source = temp.0.join("source");
        fs::create_dir(&source).unwrap();
        let outside = temp.0.join("outside");
        fs::write(&outside, b"private").unwrap();
        std::os::unix::fs::symlink(outside, source.join("linked.js")).unwrap();
        assert!(copy_package(&source, &temp.0.join("destination")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlink_at_the_destination() {
        let temp = TestDirectory::new();
        let source = temp.0.join("source");
        let destination = temp.0.join("destination");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("binding.js"), b"new").unwrap();
        let outside = temp.0.join("outside");
        fs::write(&outside, b"original").unwrap();
        std::os::unix::fs::symlink(&outside, destination.join("binding.js")).unwrap();
        assert!(copy_package(&source, &destination).is_err());
        assert_eq!(fs::read(outside).unwrap(), b"original");
    }
}
