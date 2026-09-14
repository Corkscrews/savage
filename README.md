# Savage

A cross-platform SVG viewer. Software-rendered, no GPU init, built to open from the OS as the default SVG handler.

**Stack:** [winit](https://github.com/rust-windowing/winit) + [softbuffer](https://github.com/rust-windowing/softbuffer) + [resvg](https://github.com/linebender/resvg)

## Install (macOS)

After a GitHub release exists:

```sh
brew tap corkscrews/savage https://github.com/Corkscrews/savage
brew install --cask savage
```

If macOS quarantines the download (the app is not Apple-notarized yet):

```sh
brew install --cask --no-quarantine savage
```

Publish a cask build with `./script/package-macos.sh`, then `git tag v0.1.0 && git push origin v0.1.0`. CI uploads `Savage-<version>-macos.zip` and bumps `Casks/savage.rb`.

To land in core Homebrew (`brew install --cask savage` with no tap), submit `Casks/savage.rb` to [homebrew-cask](https://github.com/Homebrew/homebrew-cask) after the app is notarized.

## Run

```sh
cargo run --release -- examples/sample.svg
```

Without a path the window stays empty until you drop an SVG onto it, or the OS asks Savage to open one.

```
Esc / Space / q    Quit
```

## Default SVG opener

The OS launches Savage and passes the file. On Linux and Windows that is `argv`. On macOS, Finder sends an Apple Event (handled in `src/platform/macos.rs`); a `.app` bundle is required.

### macOS

Prefer Homebrew (see above). From a source checkout:

```sh
./script/install.sh
```

Then: right-click any `.svg` → **Get Info** → **Open with** → Savage → **Change All**.

### Linux

```sh
cargo install --path .
cp packaging/linux/savage.desktop ~/.local/share/applications/
update-desktop-database ~/.local/share/applications
xdg-mime default savage.desktop image/svg+xml
```

### Windows

```powershell
cargo build --release
.\packaging\windows\associate.ps1 -ExePath .\target\release\savage.exe
```

Then confirm **Settings → Apps → Default apps** for `.svg` if Explorer still uses another handler.

## Startup

- No wgpu / no toolkit. First frame is a CPU blit of the SVG.
- System fonts load only when the file looks like it contains `<text>` / `<tspan>`.
- Release builds use thin LTO, one codegen unit, `panic = abort`, and symbol stripping.
