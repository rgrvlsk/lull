//! Platform implementations of the traits in [`crate::traits`].

#[cfg(target_os = "macos")]
pub mod macos;

/// Directory of the git root containing `cwd` (walks up looking for `.git`).
pub fn find_git_root(cwd: &str) -> Option<String> {
    std::path::Path::new(cwd)
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(|dir| dir.to_string_lossy().into_owned())
}
