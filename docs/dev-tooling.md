## Development & Tooling

Here's a place to share and document some tooling in use by Gitu, as well as tooling you may want to use.

### Build

There's CI that runs on every pull-request. This runs `make test`, which contains a few
extra commands over `cargo test` that you might find useful running locally.

### Testing

Gitu makes heavy use of snapshot-testing with a library called **Insta** (https://insta.rs/).
Most tests are written on a pretty high level.
The philosophy is to keep the tests easy to reason about, and make refactoring painless.

### Commit messages
Explain user-relevant changes in a `feat:`, `fix:`, `style:` or `perf:`.

Examples:
- ```feat: ability to stage all files via "Unstaged changes" section```
- ```fix: crash when opening Gitu on Windows```

For minor, technical changes that are irrelevant to the user of Gitu, you can just use any other format:
- ```refactor: changes some stuff```
- ```test: added new test about xyz```
- ```changed something```

### Changelog

The changelog is generated automatically using **git-cliff** (https://git-cliff.org/).
There's no strict commit convention, but using certain formats will create a new entry.
do:

You can see which commit formats contribute to a changelog in [cliff.toml](cliff.toml).

### Releases

In order to create a release (by a maintainer).
Run `make bump-version`. This bumps the patch version in Cargo.toml, builds, and commits.
Then publish the crate with `cargo publish`.
