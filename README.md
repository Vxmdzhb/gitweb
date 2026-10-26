# gitweb

Read-only no-JS git web frontend: argv-only (never shell) git plumbing, repo/log/diff/tree/blob/refs views, releases, and a patch/mail archive.

Part of the [astrx suite](https://github.com/Vxmdzhb/astrx-suite): one of its engines. The suite repository assembles every engine into the single `astrx` binary (`astrx gitweb`) and deploys it.

## Build

    cargo build --release --features net   # the `gitweb` binary
    cargo test --all-features

Features: `rand`, `net`. The default build uses the standard library only; `net` (and `rand`) add the suite's two vetted dependencies, tokio and getrandom. CI asserts the default build stays first-party.

## History

This repository was split out of https://github.com/Vxmdzhb/astrx-suite with the full history of its files, each followed back through its renames. The cross-check goldens are explained in [GOLDENS.md](GOLDENS.md).
