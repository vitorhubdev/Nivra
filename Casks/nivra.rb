cask "nivra" do
  version "1.0.0-nightly.20260921.44"
  sha256 "e7021c5dbd031076644426856c5fea6cc2b33ab7a7df64a963a1e993712544ab"

  url "https://github.com/vitorhubdev/Nivra/releases/download/v#{version}/nivra-v#{version}-macOS-ARM64.zip"
  name "Nivra"
  desc "Experimental native Discord client"
  homepage "https://github.com/vitorhubdev/Nivra"

  depends_on arch: :arm64
  depends_on macos: :sonoma

  app "Nivra.app"
end
