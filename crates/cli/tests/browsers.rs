//! CLI contracts, isolated from the user's Library and installed browsers.
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};

const BIN: &str = env!("CARGO_BIN_EXE_lull");
const HOST: &str = "io.github.rgrvlsk.lull";
#[allow(dead_code)]
#[path = "../src/browsers/ids.rs"]
mod ids;

fn origin() -> String {
    format!("chrome-extension://{}/", ids::CHROMIUM_EXTENSION_ID)
}

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = loop {
            let root = std::env::temp_dir().join(format!(
                "lull-browser-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => break root.canonicalize().unwrap(),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create fixture: {e}"),
            }
        };
        let fixture = Self { root };
        fs::create_dir_all(fixture.root.join("source/assets")).unwrap();
        fs::write(
            fixture.root.join("source/manifest.json"),
            "{\"manifest_version\":3}",
        )
        .unwrap();
        fs::write(
            fixture.root.join("source/assets/background.js"),
            "extension",
        )
        .unwrap();
        fs::create_dir(fixture.root.join("commands")).unwrap();
        let open = fixture.root.join("commands/open");
        fs::write(
            &open,
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$HOME/open-args\"\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(open, fs::Permissions::from_mode(0o755)).unwrap();
        fixture
    }

    fn command(&self, binary: &Path) -> Command {
        let mut cmd = Command::new(binary);
        cmd.env("HOME", &self.root)
            .env("LULL_DIR", self.root.join("lull"))
            .env("LULL_EXTENSION_DIR", self.root.join("source"))
            .env("PATH", self.root.join("commands"));
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(Path::new(BIN)).args(args).output().unwrap()
    }

    fn success(&self, args: &[&str]) -> String {
        let output = self.run(args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    fn browser(&self, support: &str) -> PathBuf {
        let dir = self.root.join("Library/Application Support").join(support);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn manifest(&self, support: &str) -> PathBuf {
        self.root
            .join("Library/Application Support")
            .join(support)
            .join("NativeMessagingHosts")
            .join(format!("{HOST}.json"))
    }

    fn read(&self, support: &str) -> Value {
        serde_json::from_slice(&fs::read(self.manifest(support)).unwrap()).unwrap()
    }

    fn doctor_line(&self, browser: &str) -> String {
        String::from_utf8(self.run(&["doctor"]).stdout)
            .unwrap()
            .lines()
            .find(|line| line.split_whitespace().nth(1) == Some(browser))
            .unwrap()
            .to_owned()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn detect_install_replace_and_uninstall() {
    let f = Fixture::new();
    for support in ["net.imput.helium", "Google/Chrome", "Mozilla"] {
        f.browser(support);
    }
    // Zen's own support directory is deliberately not a detection source.
    f.browser("zen");
    let status = f.success(&["browsers"]);
    for name in ["helium", "chrome", "firefox", "zen"] {
        assert!(status.contains(&format!("✗ {name}:")));
    }
    for name in ["brave", "edge", "vivaldi", "chromium"] {
        assert!(!status.contains(&format!("{name}:")));
    }
    assert!(f.doctor_line("helium").starts_with('✗'));
    let output = f.success(&["browsers", "install"]);
    assert!(output.contains("v1.1") && output.contains("harmless"));
    assert!(output.contains("Developer mode → Load unpacked"));
    assert!(output.contains("browser restart"));
    assert!(!f.root.join("open-args").exists());
    for support in ["net.imput.helium", "Google/Chrome", "Mozilla"] {
        let manifest = f.read(support);
        assert_eq!(manifest["name"], HOST);
        assert_eq!(manifest["description"], "lull native host");
        assert_eq!(manifest["type"], "stdio");
        let host = Path::new(manifest["path"].as_str().unwrap());
        assert!(host.is_absolute() && host.is_file());
        assert_eq!(host.file_name().unwrap(), "lull-nm-host");
        assert_eq!(
            host.canonicalize().unwrap(),
            Path::new(BIN).canonicalize().unwrap()
        );
        if support == "Mozilla" {
            assert_eq!(
                manifest["allowed_extensions"],
                json!(["lull@rgrvlsk.github.io"])
            );
            assert!(manifest.get("allowed_origins").is_none());
        } else {
            assert_eq!(manifest["allowed_origins"], json!([origin()]));
            assert!(manifest.get("allowed_extensions").is_none());
        }
    }
    assert!(f.doctor_line("helium").starts_with('✓'));
    assert!(f.doctor_line("zen").starts_with('✓'));
    let extension = f.root.join("lull/extension/chromium");
    assert_eq!(
        fs::read_to_string(extension.join("assets/background.js")).unwrap(),
        "extension"
    );
    fs::write(extension.join("obsolete.js"), "old").unwrap();
    f.success(&["browsers", "install"]);
    assert!(!extension.join("obsolete.js").exists());
    let foreign = f.manifest("Google/Chrome").with_file_name("foreign.json");
    fs::write(&foreign, "{\"name\":\"foreign.host\"}").unwrap();
    fs::write(
        f.manifest("net.imput.helium"),
        "{\"name\":\"foreign.host\"}",
    )
    .unwrap();
    f.success(&["browsers", "uninstall"]);
    assert!(foreign.exists());
    assert!(f.manifest("net.imput.helium").exists());
    assert!(!f.manifest("Google/Chrome").exists());
    assert!(!f.manifest("Mozilla").exists());
    assert!(!extension.exists());
    f.success(&["browsers", "uninstall"]);
}

#[test]
fn doctor_rejects_stale_origins_paths_and_permissions() {
    let f = Fixture::new();
    f.browser("Chromium");
    f.success(&["browsers", "install"]);
    let original = f.read("Chromium");
    let write = |manifest: &Value| {
        fs::write(
            f.manifest("Chromium"),
            serde_json::to_vec(manifest).unwrap(),
        )
        .unwrap()
    };
    let mut manifest = original.clone();
    manifest["allowed_origins"] = json!(["chrome-extension://foreign/"]);
    write(&manifest);
    assert!(f.doctor_line("chromium").starts_with('✗'));
    manifest = original.clone();
    manifest["path"] = json!(f.root.join("missing"));
    write(&manifest);
    assert!(f.doctor_line("chromium").contains("path unavailable"));
    let no_exec = f.root.join("not-executable");
    fs::write(&no_exec, "file").unwrap();
    fs::set_permissions(&no_exec, fs::Permissions::from_mode(0o644)).unwrap();
    manifest["path"] = json!(no_exec);
    write(&manifest);
    assert!(f.doctor_line("chromium").contains("not an executable"));
    fs::write(f.manifest("Chromium"), "invalid json").unwrap();
    assert!(f.doctor_line("chromium").contains("invalid manifest"));
}

#[test]
fn explicit_browser_opens_only_the_app_and_invalid_arguments_do_not_write() {
    let f = Fixture::new();
    f.browser("Google/Chrome");
    f.browser("BraveSoftware/Brave-Browser");
    for args in [
        vec!["browsers", "install", "--browser", "unknown"],
        vec!["browsers", "install", "--browser", "edge"],
        vec!["browsers", "uninstall", "--browser", "chrome"],
    ] {
        assert!(!f.run(&args).status.success());
    }
    assert!(!f.manifest("Google/Chrome").exists());
    let output = f.success(&["browsers", "install", "--browser", "chrome"]);
    assert!(output.contains("Open it manually"));
    assert_eq!(
        fs::read_to_string(f.root.join("open-args")).unwrap(),
        "-a\nGoogle Chrome\n"
    );
    assert!(!f.manifest("BraveSoftware/Brave-Browser").exists());
}

#[test]
fn missing_extension_source_prints_build_instructions() {
    let f = Fixture::new();
    f.browser("Vivaldi");
    let output = f
        .command(Path::new(BIN))
        .env("LULL_EXTENSION_DIR", f.root.join("missing"))
        .args(["browsers", "install"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("cd extension && npm run build"));
    assert!(text.contains("LULL_EXTENSION_DIR=extension/dist/chromium"));
    assert!(f.manifest("Vivaldi").exists());
}

#[test]
fn cellar_install_uses_stable_prefix_and_requires_formula_link() {
    let f = Fixture::new();
    f.browser("Microsoft Edge");
    let bin = f.root.join("brew/Cellar/lull/0.1.0/bin");
    fs::create_dir_all(&bin).unwrap();
    let exe = bin.join("lull");
    fs::copy(BIN, &exe).unwrap();
    let output = f
        .command(&exe)
        .args(["browsers", "install"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("brew/bin/lull-nm-host"));
    assert!(!bin.join("lull-nm-host").exists());
    let host = f.root.join("brew/bin/lull-nm-host");
    fs::create_dir_all(host.parent().unwrap()).unwrap();
    symlink(&exe, &host).unwrap();
    let source = bin.parent().unwrap().join("share/lull/extension/chromium");
    fs::create_dir_all(&source).unwrap();
    fs::write(source.join("manifest.json"), "brew extension").unwrap();
    let output = f
        .command(&exe)
        .env_remove("LULL_EXTENSION_DIR")
        .args(["browsers", "install"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(f.read("Microsoft Edge")["path"], json!(host));
    assert_eq!(
        fs::read_to_string(f.root.join("lull/extension/chromium/manifest.json")).unwrap(),
        "brew extension"
    );
}

#[test]
fn zen_private_directory_alone_does_not_detect_mozilla() {
    let f = Fixture::new();
    f.browser("zen");
    assert_eq!(
        f.success(&["browsers", "status"]).trim(),
        "No supported browsers detected."
    );
}

#[test]
fn dev_link_creation_and_overlapping_source_preserve_existing_files() {
    let f = Fixture::new();
    f.browser("Chromium");
    let bin = f.root.join("dev-bin");
    fs::create_dir(&bin).unwrap();
    let exe = bin.join("lull");
    fs::copy(BIN, &exe).unwrap();
    let output = f
        .command(&exe)
        .args(["browsers", "install"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(fs::read_link(bin.join("lull-nm-host")).unwrap(), exe);
    let extension = f.root.join("lull/extension/chromium");
    let output = f
        .command(&exe)
        .env("LULL_EXTENSION_DIR", &extension)
        .args(["browsers", "install"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must not overlap"));
    assert!(extension.join("manifest.json").exists());
    fs::remove_file(bin.join("lull-nm-host")).unwrap();
    symlink(f.root.join("missing-host"), bin.join("lull-nm-host")).unwrap();
    let output = f
        .command(&exe)
        .args(["browsers", "install"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("repair its lull-nm-host link"));
}

#[test]
fn install_refuses_to_replace_a_foreign_manifest_or_follow_a_symlink() {
    let f = Fixture::new();
    f.browser("Chromium");
    let path = f.manifest("Chromium");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, r#"{"name":"someone.else"}"#).unwrap();
    assert!(!f.run(&["browsers", "install"]).status.success());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        r#"{"name":"someone.else"}"#
    );

    let target = f.root.join("elsewhere.json");
    fs::write(&target, "keep").unwrap();
    fs::remove_file(&path).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert!(!f.run(&["browsers", "install"]).status.success());
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep");
}

#[test]
fn doctor_notes_browsers_with_several_profiles() {
    let f = Fixture::new();
    let dir = f.browser("Chromium");
    f.success(&["browsers", "install"]);
    fs::write(
        dir.join("Local State"),
        r#"{"profile":{"info_cache":{"Default":{},"Profile 1":{}}}}"#,
    )
    .unwrap();
    assert!(
        f.doctor_line("chromium")
            .contains("2 profiles: load the extension in each")
    );
}
