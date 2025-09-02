class UDownload < Formula
  desc "Fast, cross-platform YouTube downloader with GUI"
  homepage "https://github.com/okwareddevnest/U-Download"
  version "2.2.0"
  
  if Hardware::CPU.intel?
    url "https://github.com/okwareddevnest/U-Download/releases/download/v#{version}/u-download_#{version}_darwin_x86_64.tar.gz"
    sha256 "sha256_intel_placeholder"
  elsif Hardware::CPU.arm?
    url "https://github.com/okwareddevnest/U-Download/releases/download/v#{version}/u-download_#{version}_darwin_arm64.tar.gz"
    sha256 "sha256_arm_placeholder"
  end

  license "MIT"

  depends_on macos: ">= :big_sur"

  def install
    bin.install "u-download"
    
    # Install app bundle if available
    if File.exist?("U-Download.app")
      prefix.install "U-Download.app"
    end
    
    # Create symlink for GUI launch
    bin.write_exec_script "#{prefix}/U-Download.app/Contents/MacOS/U-Download", "u-download-gui"
  end

  def caveats
    <<~EOS
      U-Download has been installed with all required dependencies bundled.
      
      To run from terminal:
        u-download
        
      To launch GUI:
        u-download-gui
        
      Or find "U-Download" in your Applications folder.
      
      No additional dependencies (yt-dlp, aria2c, ffmpeg) need to be installed.
    EOS
  end

  test do
    system "#{bin}/u-download", "--version"
  end
end