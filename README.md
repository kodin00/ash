# 🔥 ash

[![CI](https://github.com/kodin00/ash/actions/workflows/ci.yml/badge.svg)](https://github.com/kodin00/ash/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/kodin00/ash)](https://github.com/kodin00/ash/releases/latest)
[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

`ssh`, minus the memory test.

Name a machine once. Connect to it forever.

```console
$ ash add
? Machine alias: prod
? SSH user: root
? IP address or hostname: 10.0.0.8
? SSH authentication: Private key: ~/.ssh/id_ed25519

$ ash prod
```

## ⚡ Install

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/kodin00/ash/releases/latest/download/ash-installer.sh | sh
```

macOS and Linux. Intel and ARM. No Rust required.

## 🖥 Use

```sh
ash add                                        # interactive setup
ash add prod root:10.0.0.8 ~/.ssh/id_ed25519  # one-line setup
ash list                                       # list machines
ash prod                                       # connect
ash remove prod                                # remove
```

## 🔑 Auth

Pick a key from `~/.ssh`, enter another path, or use your SSH agent/password.
Passwords are never stored.

## 📁 Config

```text
~/.config/ash/config
```

`$XDG_CONFIG_HOME/ash/config` and `$ASH_CONFIG_FILE` override the default.

```sh
config="${ASH_CONFIG_FILE:-${XDG_CONFIG_HOME:-$HOME/.config}/ash/config}"
"${EDITOR:-vi}" "$config"
```

## 🛠 Build

```sh
mise install
mise run check
```

## 🚀 Release

```sh
git tag v0.1.0
git push origin master --tags
```