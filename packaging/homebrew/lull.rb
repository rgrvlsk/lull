# Source of truth for Formula/lull.rb in rgrvlsk/homebrew-tap (ADR-0008). The release
# steps in packaging/RELEASE.md fill in the version and both sha256 values.
class Lull < Formula
  desc "Stops the dev servers your coding agents forgot"
  homepage "https://github.com/rgrvlsk/lull"
  url "https://github.com/rgrvlsk/lull/archive/refs/tags/v@VERSION@.tar.gz"
  sha256 "@SOURCE_SHA256@"
  license any_of: ["MIT", "Apache-2.0"]

  depends_on "rust" => :build
  depends_on :macos

  # The unpacked Chromium extension, built from extension/ at the same tag (ADR-0007).
  resource "chromium-extension" do
    url "https://github.com/rgrvlsk/lull/releases/download/v@VERSION@/lull-extension-chromium-@VERSION@.zip"
    sha256 "@EXTENSION_SHA256@"
  end

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/cli")
    # Native messaging forbids arguments in the manifest path (contract §3).
    bin.install_symlink "lull" => "lull-nm-host"
    (share/"lull/extension/chromium").install resource("chromium-extension")
  end

  # launchd does not create the directory of a service's log path.
  def post_install
    (Pathname.new(Dir.home)/"Library/Logs/lull").mkpath
  end

  service do
    run [opt_bin/"lull", "daemon"]
    keep_alive true
    process_type :background
    log_path "#{Dir.home}/Library/Logs/lull/daemon.log"
    error_log_path "#{Dir.home}/Library/Logs/lull/daemon.log"
  end

  def caveats
    <<~EOS
      Start the daemon (it stops idle agent-started dev servers after 30 minutes):
        brew services start lull
      Then connect your Chromium browsers (Helium, Chrome, Brave, Edge, Vivaldi):
        lull browsers install
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/lull --version")
    assert_predicate bin/"lull-nm-host", :symlink?
  end
end
