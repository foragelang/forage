# Install

Three ways to get the `forage` CLI on macOS.

## Homebrew

```sh
brew install foragelang/forage/forage
```

`brew upgrade foragelang/forage/forage` later to pick up new releases.

## curl

```sh
curl -fsSL https://foragelang.com/install.sh | sh
```

Installs to `~/.local/bin/forage`. Override the destination with `FORAGE_INSTALL_DIR=...`. Pin a specific version with `FORAGE_VERSION=v0.1.0`.

If `~/.local/bin` is not on your `PATH`, the installer prints the line to add to `~/.zshrc`.

## Build from source

```sh
git clone https://github.com/foragelang/forage
cd forage
swift build -c release
sudo cp .build/release/forage /usr/local/bin/
```

Requires Xcode 16+ on macOS 14+.

## Toolkit (macOS app)

The Toolkit is an interactive recipe authoring app that wraps a WKWebView, captures fetch/XHR traffic, and publishes recipes to the hub.

Download the latest signed DMG from [GitHub Releases](https://github.com/foragelang/forage/releases/latest). Mount, drag `Toolkit.app` to `/Applications`, launch.

If the DMG for the current release is ad-hoc signed (no Developer ID signature), macOS will block the first launch. Right-click `Toolkit.app` in `/Applications` and choose **Open** to allow it.

## Verify the install

```sh
forage --version
forage --help
```

Then walk through [Getting started](/docs/getting-started) for the first recipe.
