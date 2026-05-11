# This is the canonical source of the `forage` Homebrew formula.
#
# To publish:
#   1. Create the foragelang/homebrew-tap GitHub repository (empty).
#   2. Copy this file to foragelang/homebrew-tap/Formula/forage.rb.
#   3. Push.
#
# Once the tap repo exists, `update-homebrew-tap` in
# .github/workflows/release.yml will keep `url` + `sha256` fresh on every
# release (gated on the HOMEBREW_TAP_TOKEN secret + ENABLE_HOMEBREW_TAP_UPDATE
# repo variable). Until then, edit the `url` + `sha256` here by hand after
# each release and mirror to the tap repo.

class Forage < Formula
  desc "Declarative scraping platform — CLI for the forage runtime"
  homepage "https://foragelang.com"
  url "https://github.com/foragelang/forage/releases/download/v0.1.0/forage-v0.1.0-macos.tar.gz"
  sha256 "PLACEHOLDER_SHA256_FROM_RELEASE"
  license "MIT"

  depends_on macos: :sonoma

  def install
    bin.install "forage"
  end

  test do
    assert_match "forage", shell_output("#{bin}/forage --help")
  end
end
