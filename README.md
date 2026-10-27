# gitweb

Read-only no-JS git web frontend: argv-only (never shell) git plumbing, repo/log/diff/tree/blob/refs views, releases, and a patch/mail archive.

Part of the [astrx suite](https://github.com/Vxmdzhb/astrx-suite): one of its engines. The suite repository assembles every engine into the single `astrx` binary (`astrx gitweb`) and deploys it.

## JSON API

Everything the pages show is also available as JSON, for a client that renders
it itself (the AstrX site's git module uses it):

    /repos.json                      every repository
    /<repo>/summary.json             default branch, branches, tags, latest commits, clone URL
    /<repo>/tree.json?ref=&path=     a directory, with its README as text
    /<repo>/blob.json?ref=&path=     one file as text (or binary: true)
    /<repo>/log.json?ref=&page=      a page of the log
    /<repo>/commit.json?id=          one commit with its diff

It is read-only, takes the same validated refs and paths as the pages, and
answers errors as `{"error": "..."}`.

## Build

    cargo build --release --features net   # the `gitweb` binary
    cargo test --all-features

Features: `rand`, `net`. The default build uses the standard library only; `net` (and `rand`) add the suite's two vetted dependencies, tokio and getrandom. CI asserts the default build stays first-party.

## History

This repository was split out of https://github.com/Vxmdzhb/astrx-suite with the full history of its files, each followed back through its renames. The cross-check goldens are explained in [GOLDENS.md](GOLDENS.md).

## License

Released under the [Do Whatever License](LICENSE), with a code of conduct
in the same spirit: [CODE_OF_CONDUCT](CODE_OF_CONDUCT).
