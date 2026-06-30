class Polymarket < Formula
  desc "CLI for Polymarket — browse markets, trade, and manage positions"
  homepage "https://github.com/jesodium/fiberglass"
  version "0.1.12"
  license "MIT"

  on_macos do
    on_intel do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/polymarket-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "e309373f6a06a4d0fce7c81c0553f09ee36460b3ee5559884a81349a3a56cbf0"
    end

    on_arm do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/polymarket-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "18e25f7dcc35b221f83bde6d19d18b2f0985e0b065a87dabdfee7a753e9d493f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/polymarket-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "ea90b2cff548d510617d60d7c68c9dd1d55dd9904bc1b7dca68f385066927ebb"
    end

    on_arm do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/polymarket-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "fb6b1d30efe44c7330f94e483b35e06fb70d8dff58ffd90a8d2ed9f84c0ba2d1"
    end
  end

  def install
    bin.install "polymarket"
  end

  test do
    assert_match "polymarket", shell_output("#{bin}/polymarket --version")
  end
end
