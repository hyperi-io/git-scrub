# Project:   git-scrub
# File:      packaging/homebrew/git-scrub.rb
# Purpose:   Homebrew formula template for git-scrub, rendered into
#            hyperi-io/homebrew-tap Formula/git-scrub.rb by each release
# Language:  Ruby
#
# License:   Apache-2.0
# Copyright: (c) 2026 HYPERI PTY LIMITED

class GitScrub < Formula
  desc "Surgical removal of unwanted content from git history"
  homepage "https://github.com/hyperi-io/git-scrub"
  version "${VERSION}"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/hyperi-io/git-scrub/releases/download/v${VERSION}/git-scrub-${VERSION}-darwin-arm64.tar.gz"
      sha256 "${SHA256_DARWIN_ARM64}"
    else
      url "https://github.com/hyperi-io/git-scrub/releases/download/v${VERSION}/git-scrub-${VERSION}-darwin-amd64.tar.gz"
      sha256 "${SHA256_DARWIN_AMD64}"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/hyperi-io/git-scrub/releases/download/v${VERSION}/git-scrub-${VERSION}-linux-arm64.tar.gz"
      sha256 "${SHA256_LINUX_ARM64}"
    else
      url "https://github.com/hyperi-io/git-scrub/releases/download/v${VERSION}/git-scrub-${VERSION}-linux-amd64.tar.gz"
      sha256 "${SHA256_LINUX_AMD64}"
    end
  end

  def install
    bin.install "git-scrub"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/git-scrub --version")
  end
end
