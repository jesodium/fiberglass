class Polymarket < Formula
  desc "CLI for Polymarket — browse markets, trade, and manage positions"
  homepage "https://github.com/jesodium/polymarket-cli"
  version "0.1.6"
  license "MIT"

  on_macos do
    on_intel do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "714668b36e28723100424a2951a11325df5fe51be2b68df33c536517ef8db0ad"
    end

    on_arm do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "b79af6673df6c2133f460fd4a8b31fd219b74a3d1e78de928f644cdb71ed714b"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "f4e2920a715c61fa2655026af827953a25c64e1e43e77ea7a900972d0f31fc4a"
    end

    on_arm do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0f0e55ebee29e397f03a5de75c002a501b56685aa26579f5f06324c8e8ea76ae"
    end
  end

  def install
    bin.install "polymarket"
  end

  test do
    assert_match "polymarket", shell_output("#{bin}/polymarket --version")
  end
end
