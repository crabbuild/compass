# Contribute to Compass

Compass welcomes bug fixes, documentation improvements, tests, performance work, and focused features. This guide explains where to start and what maintainers expect from a contribution.

## Choose the right channel

Use the channel that matches your goal:

- Ask usage questions and discuss open-ended ideas in [GitHub Discussions](https://github.com/crabbuild/compass/discussions)
- Report reproducible bugs through the [issue chooser](https://github.com/crabbuild/compass/issues/new/choose)
- Submit actionable feature proposals through the feature request form
- Report vulnerabilities through [GitHub private vulnerability reporting](https://github.com/crabbuild/compass/security/advisories/new)
- Follow the [Code of Conduct](CODE_OF_CONDUCT.md) in every project space

Don't publish credentials, private source code, exploit details, or other sensitive data in an issue or discussion.

## Set up the workspace

Compass requires Rust 1.97.1 or newer. The repository pins its toolchain and required components in `rust-toolchain.toml`.

```bash
git clone https://github.com/crabbuild/compass.git
cd compass
cargo build --workspace --locked
```

Run the command-line interface from the workspace while developing:

```bash
cargo run --locked -p compass-cli -- --help
```

Python isn't required to build Compass. Differential compatibility tests need a sibling Graphify checkout and the interpreters described in [COMPATIBILITY.md](COMPATIBILITY.md).

## Make a focused change

Before editing, search existing issues and discussions for related work. Describe the behavior you want to change, then keep the patch scoped to that behavior.

Follow these project conventions:

- Preserve stable output, identifiers, and file formats unless the change includes an approved migration
- Add or update tests for behavior changes
- Update user documentation when commands, flags, outputs, limits, or security boundaries change
- Keep credentials, generated artifacts, local caches, and unrelated formatting out of the patch
- Preserve third-party attribution and license notices

## Verify your change

Run checks that match the files you changed. A complete Rust verification pass uses:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
```

For documentation-only changes, run `git diff --check` and verify every changed link and command. Include any check you couldn't run in the pull request description.

## Submit a pull request

Open a pull request with a concise title and a description that covers:

- The problem or user need
- The chosen approach
- User-visible and compatibility effects
- Tests or checks you ran
- Documentation and migration updates

Keep commits reviewable and avoid mixing unrelated changes. Maintainers may ask you to split a broad pull request before review.

## License contributions

Compass is available under either the [MIT License](LICENSE-MIT) or [Apache License 2.0](LICENSE-APACHE). Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in Compass uses the same dual license without additional terms or conditions.
