class Fiberglass < Formula
  desc "Fiberglass — a trading terminal for Polymarket"
  homepage "https://github.com/jesodium/fiberglass"
  version "0.1.18"
  license "MIT"

  on_macos do
    on_intel do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "76411c03c4c2777ecfffe70a48b5299d062da55406d09730d8d6ce42d53afff3"
    end

    on_arm do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "60ff0ff95037da6e4f1d7f6a73544b2b1a1d938bf84fa7a775df068811243bf5"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "5b2142d0ee921f7551e5ef6da656a9bf3dc242db708f4dc59bcf6d1f5eb2b843"
    end

    on_arm do
      url "https://github.com/jesodium/fiberglass/releases/download/v#{version}/fiberglass-v#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "89762889e93bafe4f4655cd04a0c86d850b8cfe8b1e20c64bee83de37572f71a"
    end
  end

  def install
    bin.install "fiberglass"
  end

  test do
    assert_match "fiberglass", shell_output("#{bin}/fiberglass --version")
  end
end
