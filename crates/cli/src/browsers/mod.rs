//! Browser native-messaging registration and unpacked Chromium extension installation.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::{Value, json};

use crate::paths::{Paths, home};

/// Native-messaging host name (§14).
pub const NM_HOST_NAME: &str = "io.github.rgrvlsk.lull";

/// Chromium extension id, pinned by the manifest `key` (ADR-0007).
// ponytail: placeholder until WP-R generates the key pair (needs the author's go-ahead);
// nm-host and the NM manifests both read this one constant.
pub const CHROMIUM_EXTENSION_ID: &str = "lullplaceholderlullplaceholderaa";

/// Firefox/Zen add-on id (v1.1, §9).
pub const FIREFOX_ADDON_ID: &str = "lull@rgrvlsk.github.io";

struct Browser {
    name: &'static str,
    app: &'static str,
    support: &'static str,
    mozilla: bool,
}

const BROWSERS: &[Browser] = &[
    Browser {
        name: "helium",
        app: "Helium",
        support: "net.imput.helium",
        mozilla: false,
    },
    Browser {
        name: "chrome",
        app: "Google Chrome",
        support: "Google/Chrome",
        mozilla: false,
    },
    Browser {
        name: "brave",
        app: "Brave Browser",
        support: "BraveSoftware/Brave-Browser",
        mozilla: false,
    },
    Browser {
        name: "edge",
        app: "Microsoft Edge",
        support: "Microsoft Edge",
        mozilla: false,
    },
    Browser {
        name: "vivaldi",
        app: "Vivaldi",
        support: "Vivaldi",
        mozilla: false,
    },
    Browser {
        name: "chromium",
        app: "Chromium",
        support: "Chromium",
        mozilla: false,
    },
    Browser {
        name: "firefox",
        app: "Firefox",
        support: "Mozilla",
        mozilla: true,
    },
    Browser {
        name: "zen",
        app: "Zen",
        support: "Mozilla",
        mozilla: true,
    },
];

impl Browser {
    fn support_dir(&self) -> PathBuf {
        PathBuf::from(home())
            .join("Library/Application Support")
            .join(self.support)
    }

    fn manifest(&self) -> PathBuf {
        self.support_dir()
            .join("NativeMessagingHosts")
            .join(format!("{NM_HOST_NAME}.json"))
    }
}

fn detected() -> Vec<&'static Browser> {
    BROWSERS
        .iter()
        .filter(|b| b.support_dir().is_dir())
        .collect()
}

pub fn run(args: Vec<String>) -> ExitCode {
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] | ["status"] => {
            let lines = doctor_lines();
            if lines.is_empty() {
                println!("No supported browsers detected.");
            }
            for (label, ok, text) in lines {
                println!("{} {label}: {text}", if ok { "✓" } else { "✗" });
            }
            Ok(())
        }
        ["install"] => install(None),
        ["install", "--browser", name] => install(Some(name)),
        ["uninstall"] => uninstall(),
        _ => {
            eprintln!("usage: lull browsers [status|install [--browser <name>]|uninstall]");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("lull browsers: {e}");
            ExitCode::FAILURE
        }
    }
}

fn nm_host(exe: &Path) -> Result<PathBuf, String> {
    let bin = exe.parent().ok_or("executable has no parent directory")?;
    // current_exe resolves Homebrew's bin symlink to the versioned Cellar binary.
    let cellar = bin.parent().and_then(Path::parent).and_then(Path::parent);
    let brew = cellar.filter(|cellar| {
        cellar.file_name().is_some_and(|n| n == "Cellar")
            && bin.file_name().is_some_and(|n| n == "bin")
            && bin
                .parent()
                .and_then(Path::parent)
                .and_then(Path::file_name)
                .is_some_and(|n| n == "lull")
    });
    let host = match brew {
        Some(cellar) => cellar
            .parent()
            .ok_or("Cellar has no prefix")?
            .join("bin/lull-nm-host"),
        None => bin.join("lull-nm-host"),
    };
    if brew.is_none()
        && fs::symlink_metadata(&host).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        // Another `lull browsers install` may have created it meanwhile.
        match symlink(exe, &host) {
            Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => {
                return Err(format!("create {}: {e}", host.display()));
            }
            _ => {}
        }
    }
    if !host.is_file() {
        return Err(format!(
            "native host {} does not exist as a file; reinstall lull or repair its lull-nm-host link",
            host.display()
        ));
    }
    Ok(host)
}

fn install(name: Option<&str>) -> Result<(), String> {
    let browsers = if let Some(name) = name {
        let browser = BROWSERS.iter().find(|b| b.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| format!("unknown browser {name}; use helium, chrome, brave, edge, vivaldi, chromium, firefox or zen"))?;
        if !browser.support_dir().is_dir() {
            return Err(format!(
                "{} not detected at {}",
                browser.name,
                browser.support_dir().display()
            ));
        }
        vec![browser]
    } else {
        detected()
    };
    if browsers.is_empty() {
        println!("No supported browsers detected.");
        return Ok(());
    }
    let exe = std::env::current_exe().map_err(|e| format!("locate executable: {e}"))?;
    let host = nm_host(&exe)?;
    for browser in &browsers {
        let mut manifest = json!({
            "name": NM_HOST_NAME, "description": "lull native host", "path": host,
            "type": "stdio"
        });
        if browser.mozilla {
            manifest["allowed_extensions"] = json!([FIREFOX_ADDON_ID]);
        } else {
            manifest["allowed_origins"] =
                json!([format!("chrome-extension://{CHROMIUM_EXTENSION_ID}/")]);
        }
        let path = browser.manifest();
        fs::create_dir_all(path.parent().unwrap())
            .map_err(|e| format!("create manifest directory: {e}"))?;
        fs::write(
            &path,
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("write {}: {e}", path.display()))?;
        println!("{}: installed {}", browser.name, path.display());
    }
    if browsers.iter().any(|b| b.mozilla) {
        println!(
            "Firefox/Zen extension ships in v1.1. Writing the shared Mozilla manifest is harmless and lets lull doctor report it."
        );
    }
    if browsers.iter().any(|b| !b.mozilla) {
        let source = std::env::var_os("LULL_EXTENSION_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                exe.parent()
                    .unwrap()
                    .join("../share/lull/extension/chromium")
            });
        let destination = Paths::from_env().dir.join("extension/chromium");
        if source.is_dir() {
            // Resolve before removal, including when a caller selects the installed copy.
            let source = source
                .canonicalize()
                .map_err(|e| format!("locate extension source: {e}"))?;
            if destination.exists() {
                let dest = destination
                    .canonicalize()
                    .map_err(|e| format!("locate installed extension: {e}"))?;
                if source.starts_with(&dest) || dest.starts_with(&source) {
                    return Err("extension source and destination must not overlap".into());
                }
                fs::remove_dir_all(&destination)
                    .map_err(|e| format!("remove old extension: {e}"))?;
            }
            copy_dir(&source, &destination)?;
        } else {
            println!(
                "Extension source not found at {}. Build it with: cd extension && npm run build",
                source.display()
            );
            println!(
                "Then run with LULL_EXTENSION_DIR=extension/dist/chromium lull browsers install"
            );
        }
        println!(
            "In the browser's Extensions page: Developer mode → Load unpacked → {}",
            destination.display()
        );
        println!("Chromium extension updates need a browser restart.");
    }
    if name.is_some() {
        let browser = browsers[0];
        match Command::new("open").args(["-a", browser.app]).status() {
            Ok(status) if status.success() => {}
            _ => println!(
                "Could not open {}. Open it manually and follow the instructions above.",
                browser.app
            ),
        }
    }
    Ok(())
}

fn copy_dir(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|e| format!("create {}: {e}", destination.display()))?;
    for entry in fs::read_dir(source).map_err(|e| format!("read {}: {e}", source.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let target = destination.join(entry.file_name());
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)
                .map_err(|e| format!("copy {}: {e}", entry.path().display()))?;
        } else {
            return Err(format!(
                "extension source {} is not a regular file or directory",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

fn uninstall() -> Result<(), String> {
    for browser in detected() {
        let path = browser.manifest();
        if fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .is_some_and(|manifest| manifest["name"] == NM_HOST_NAME)
        {
            fs::remove_file(&path).map_err(|e| format!("remove {}: {e}", path.display()))?;
            println!("Removed {}", path.display());
        }
    }
    let extension = Paths::from_env().dir.join("extension/chromium");
    if extension.exists() {
        fs::remove_dir_all(&extension)
            .map_err(|e| format!("remove {}: {e}", extension.display()))?;
        println!("Removed {}", extension.display());
    }
    Ok(())
}

/// `lull doctor` lines `(label, ok, text)` for each detected browser's NM manifest.
pub fn doctor_lines() -> Vec<(String, bool, String)> {
    detected()
        .into_iter()
        .map(|browser| {
            let path = browser.manifest();
            let result = check_manifest(browser, &path);
            let ok = result.is_ok();
            let text = match result {
                Ok(()) => format!("manifest ready ({})", path.display()),
                Err(e) => format!("{e} ({})", path.display()),
            };
            (browser.name.into(), ok, text)
        })
        .collect()
}

fn check_manifest(browser: &Browser, path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("manifest missing or unreadable: {e}"))?;
    let manifest: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid manifest: {e}"))?;
    if manifest["name"] != NM_HOST_NAME || manifest["type"] != "stdio" {
        return Err("manifest name or type does not match lull".into());
    }
    let (field, allowed) = if browser.mozilla {
        ("allowed_extensions", json!([FIREFOX_ADDON_ID]))
    } else {
        (
            "allowed_origins",
            json!([format!("chrome-extension://{CHROMIUM_EXTENSION_ID}/")]),
        )
    };
    if manifest[field] != allowed {
        return Err(format!("manifest {field} does not match lull"));
    }
    let host = manifest["path"]
        .as_str()
        .ok_or("manifest has no native host path")?;
    if !Path::new(host).is_absolute() {
        return Err("native host path is not absolute".into());
    }
    let metadata = fs::metadata(host).map_err(|e| format!("native host path unavailable: {e}"))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err("native host is not an executable file".into());
    }
    Ok(())
}
