class Fiberglass < Formula
  desc "Fiberglass — a trading terminal for Polymarket"
  homepage "https://github.com/jesodium/fiberglass"
  version "0.1.19"
  license "MIT"

  on_macos do
    on_intel do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "14d88b6b8df7fa6bae7a1fd2148cedc833cf8df3a60c08d1816fca0a32d1e7f8"
    end

    on_arm do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "3d32082fd2ab1801ff35f97c83cf3f89ae3749beb2e2484ee6c60f70e5393a2e"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "b60d4bf7f4ea23f5ddc6502545a6eec6a5e0a7a0f5e2d33857c2794d2ee87131"
    end

    on_arm do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "f4b5741a1d9354943e3dc84739fb8dcee4db1dd48250ed0fac1462423b0fbf67"
    end
  end

  def install
    bin.install "fiberglass"
  end

  test do
    assert_match "fiberglass", shell_output("#{bin}/fiberglass --version")
  end
end
