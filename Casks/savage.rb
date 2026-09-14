cask "savage" do
  version "0.1.0"
  sha256 :no_check

  url "https://github.com/Corkscrews/savage/releases/download/v#{version}/Savage-#{version}-macos.zip",
      verified: "github.com/Corkscrews/savage/"
  name "Savage"
  desc "Fast, lightweight SVG viewer"
  homepage "https://github.com/Corkscrews/savage"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :big_sur"

  app "Savage.app"

  zap trash: [
    "~/Library/Preferences/dev.savage.viewer.plist",
    "~/Library/Saved Application State/dev.savage.viewer.savedState",
  ]
end
