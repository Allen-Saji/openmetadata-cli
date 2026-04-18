# Draft Homebrew formula for the omd CLI.
#
# To ship: create the tap repo `Allen-Saji/homebrew-tap`, drop this file at
# `Formula/omd.rb`, and update the six SHA256 placeholders after the first
# v0.7.0 release. The release workflow publishes `*.sha256` files alongside
# each tarball, so `shasum -a 256 <asset>` or the `.sha256` contents work.
class Omd < Formula
  desc "Command-line tool for OpenMetadata"
  homepage "https://github.com/Allen-Saji/openmetadata-cli"
  version "0.7.0"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/Allen-Saji/openmetadata-cli/releases/download/v#{version}/omd-v#{version}-aarch64-darwin.tar.gz"
      sha256 "REPLACE_WITH_AARCH64_DARWIN_SHA256"
    else
      url "https://github.com/Allen-Saji/openmetadata-cli/releases/download/v#{version}/omd-v#{version}-x86_64-darwin.tar.gz"
      sha256 "REPLACE_WITH_X86_64_DARWIN_SHA256"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/Allen-Saji/openmetadata-cli/releases/download/v#{version}/omd-v#{version}-aarch64-linux.tar.gz"
      sha256 "REPLACE_WITH_AARCH64_LINUX_SHA256"
    else
      url "https://github.com/Allen-Saji/openmetadata-cli/releases/download/v#{version}/omd-v#{version}-x86_64-linux.tar.gz"
      sha256 "REPLACE_WITH_X86_64_LINUX_SHA256"
    end
  end

  def install
    bin.install "omd"
    generate_completions_from_executable(bin/"omd", "completions")
  end

  test do
    assert_match(/^omd \d+\.\d+\.\d+/, shell_output("#{bin}/omd --version"))
  end
end
