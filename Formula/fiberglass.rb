class Fiberglass < Formula
  desc "Fiberglass — a trading terminal for Polymarket"
  homepage "https://github.com/jesodium/fiberglass"
  version "0.1.15"
  license "MIT"

  on_macos do
    on_intel do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "9c2748368579503a0211c33db9df23beb7f91a1bacf7a64ba8ac57c6e4abf243"
    end

    on_arm do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "5f61e1d131b84b737b546d07716f0563eee8476e835b25748418fdc8a35d2fe0"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "883a2de9a27f5a4f4b82090de1a86db4df793798a0f464b822cea16ec5e4a701"
    end

    on_arm do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "8c883008abb85fb3cb7da32783c7082b289c9316602e05810fcfb8d8616ace72"
    end
  end

  def install
    bin.install "fiberglass"
  end

  test do
    assert_match "fiberglass", shell_output("#{bin}/fiberglass --version")
  end
end
