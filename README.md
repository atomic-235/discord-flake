# discord-flake

Discord REST CLI, run under the `with-nono personal` credential proxy: the
process only ever holds a phantom `DISCORD_TOKEN` and the proxy substitutes the
real user token on requests to `discord.com`.

## Commands

```
discord me                                       # account id, email, mfa state
discord guilds [--limit N]                       # servers: id, name
discord channels <guild> [--type text|voice|...] # channels in a server
discord dms [--limit N]                          # DM channels
discord messages <channel> [--limit N] [--before ID] [--page N]
discord search <guild> <query> [--limit N] [--page N]
discord send <channel> <text...>
```

`<channel>` accepts an ID, a channel name (with or without `#`), or
`guild/channel`. `<guild>` accepts an ID or a name substring. Every command
accepts `--json`.

## Build

```sh
nix build .#default     # fetches the published static release asset
nix build .#static      # builds the static binary from source (CI does this)
```

Releases are produced by `.github/workflows/release.yml` on every push to
`main` that changes the crate.
