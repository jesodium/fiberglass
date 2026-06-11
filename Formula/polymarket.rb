class Polymarket < Formula
  desc "CLI for Polymarket — browse markets, trade, and manage positions"
  homepage "https://github.com/jesodium/polymarket-cli"
  version "0.1.7"
  license "MIT"

  on_macos do
    on_intel do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "ecf9683716c2705d40bfce41af16ff529ea47c5a00db72e5798fd6610a606f05"
    end

    on_arm do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "40876f658189ce74b26ec1aa0a619bf094a1534e234741a594ef89163f3a522f"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "68f4399da3a505c08aa4374adf27d940eadc8973546d3fec28e5487ce9882784"
    end

    on_arm do
      url "https://github.com/jesodium/polymarket-cli/releases/download/v#{version}/polymarket-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "0e07ca5ad0d0f83987ee37e526f25e2565d6144fec7f343cd0db29be33675177"
    end
  end

  def install
    bin.install "polymarket"
  end

  test do
    assert_match "polymarket", shell_output("#{bin}/polymarket --version")
  end
end
