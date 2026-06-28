class Polymarket < Formula
  desc "CLI for Polymarket — browse markets, trade, and manage positions"
  homepage "https://github.com/jesodium/polymarket-cli"
  version "0.1.10"
  license "MIT"

  on_macos do
    on_intel do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "d0693ac14b9f92f48e7c25c2d311f32180b8a39d6d580d58ceb432b80c207fe3"
    end

    on_arm do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "1c8d039f46a8782df5dd0a5f85b44a677fd1fec5c2d113a9a675602f96f73ba6"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "0d04a790be1526ee5568d6d05e8fddc7de3355caf194f68d0249c1d21af20ffb"
    end

    on_arm do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "14782f61d46181c29770b619b02a8bfc48a98ad8de48ec2448ebae4fb89142ef"
    end
  end

  def install
    bin.install "polymarket"
  end

  test do
    assert_match "polymarket", shell_output("#{bin}/polymarket --version")
  end
end
