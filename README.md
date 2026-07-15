# ash

`ssh`, minus the memory test.

```console
$ ash add
? Machine alias: prod
? SSH user: root
? IP address or hostname: 10.0.0.8
? SSH authentication: Private key: ~/.ssh/id_ed25519

$ ash prod
```

## Install

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/kodin00/ash/releases/latest/download/ash-installer.sh | sh
```

Prebuilt for macOS and Linux, on Intel and ARM. Installs to `~/.local/bin`.

## Use

```sh
ash add                                        # interactive setup
ash add prod root:10.0.0.8 ~/.ssh/id_ed25519  # one-line setup
ash list                                       # saved machines
ash prod                                       # connect
ash remove prod                                # remove
```

No key selected? OpenSSH uses your agent or asks for a password. Passwords are
never stored.

## Build

```sh
mise install
mise run check
```

## Release

```sh
git tag v0.1.0
git push origin master --tags
```
