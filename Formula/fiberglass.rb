class Fiberglass < Formula
  desc "Fiberglass — a trading terminal for Polymarket"
  homepage "https://github.com/jesodium/fiberglass"
  version "0.1.21"
  license "MIT"

  on_macos do
    on_intel do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "829b5043a08b46ee20ca88d289c22cbf0ac8ff58421fa1c6ea53ee6ba022fbf8"
    end

    on_arm do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "55d9946881d2ba4b2c6e372cc4ce4b7cc0f23ee4e2929bb85a482e8f906083f5"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "1351df524fa319788ad414b42d02bcb6929e5609141563e98ecb0d38a68b6100"
    end

    on_arm do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "7b4eebe44aa2ca62d7bfbd97108bee2a134493a5efb69ca60a7a7f5b143c09d3"
    end
  end

  def install
    bin.install "fiberglass"
  end

  test do
    assert_match "fiberglass", shell_output("#{bin}/fiberglass --version")
  end
end
