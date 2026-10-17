## Development & Tooling

Here's a place to share and document some tooling in use by Gitu, as well as tooling you may want to use.

### Build

There's CI that runs on every pull-request. This runs `make test`, which contains a few
extra commands over `cargo test` that you might find useful running locally.

### Testing

Gitu makes heavy use of snapshot-testing with a library called **Insta** (https://insta.rs/).
Most tests are written on a pretty high level.
The philosophy is to keep the tests easy to reason about, and make refactoring painless.

### Releases

In order to create a release (by a maintainer).
Run `make bump-version`. This bumps the patch version in Cargo.toml, builds, and commits.
Then publish the crate with `cargo publish`.
