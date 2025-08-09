# Working on Rudy

Rudy is still a very young project. The current state of contributions is:
I will happily accept any contributions from idle thoughts, to suggested changes, to full on PRs.

However, expect there to be large breaking changes from time to time, and expect minimal documentation
or guides on how to contribute.

## Testing

This repository is a mostly vanilla Rust project. The one exception is testing.

We want to be cross-platform compatible, and so some work has been put into making
the testing infrastructure support this.

The way we do this is via [`xtask` commands](https://github.com/matklad/cargo-xtask)
that can build examples for multiple platforms.

For reproducibility, these artifacts are published via GitHub releases.

The tl;dr is that to test, you should first run `cargo xtask download-artifacts` to get
the latest, and then `cargo insta test` will run snapshot tests using [insta](https://insta.rs/)
using those artifacts.

Most of the `xtask` setup was purpose built for myself -- which means it mostly assumes that your
host OS is macOS arm64. However, testing etc should work fine on Linux, there are just a few
places where you wont get updated snapshots for macOS.

The test snapshots are also likely dependent on a specific Rust version. We generally try and keep the
version up to date with the latest Rust stable release.
