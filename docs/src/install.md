# Install

## macOS

**Homebrew (recommended once R11 stable):**

```sh
brew install foragelang/forage/forage
```

**Curl pipe:**

```sh
curl -fsSL https://foragelang.com/install.sh | sh
```

**Direct binary:**

```sh
curl -L https://github.com/foragelang/forage/releases/latest/download/forage-v0.1.0-aarch64-apple-darwin.tar.gz \
  | tar -xz
mv forage /usr/local/bin/
forage --version
```

**Forage Studio (when R9 ships):** download the signed `.dmg` from
[GitHub Releases](https://github.com/foragelang/forage/releases/latest)
and drag `Forage Studio.app` into `/Applications`.

## Linux

```sh
curl -L https://github.com/foragelang/forage/releases/latest/download/forage-v0.1.0-x86_64-unknown-linux-gnu.tar.gz \
  | tar -xz
mv forage ~/.local/bin/
```

For Forage Studio (when R9 ships), grab the AppImage or `.deb`.

## Windows

```powershell
Invoke-WebRequest `
  -Uri "https://github.com/foragelang/forage/releases/latest/download/forage-v0.1.0-x86_64-pc-windows-msvc.zip" `
  -OutFile forage.zip
Expand-Archive forage.zip .
.\forage.exe --version
```

## Building from source

You need Rust 1.85 or later.

```sh
git clone https://github.com/foragelang/forage
cd forage
cargo build --release --bin forage
./target/release/forage --version
```

For Linux desktop dependencies (needed by the optional `live` feature
of `forage-browser`):

```sh
sudo apt install \
  libwebkit2gtk-4.1-dev \
  libgtk-3-dev \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libsoup-3.0-dev
```
