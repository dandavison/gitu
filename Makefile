.PHONY: *

test:
	cargo insta test --unreferenced reject
	cargo bench --no-run
	cargo clippy -- -Dwarnings
	cargo fmt --check
  # FIXME cargo deny complaining about vulnerabilities
	# cargo deny check
	git cliff --unreleased

flamegraph:
	cargo flamegraph --profile profiling --bin gitu

heaptrack:
	cargo build --profile profiling
	heaptrack target/profiling/gitu

test-depend:
	cargo install cargo-insta git-cliff || true

bump-version:
	next=$$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1 | awk -F. '{print $$1"."$$2"."$$3+1}') && \
	perl -pi -e 'if (!$$d && s/^version = "[^"]+"/version = "'$$next'"/) { $$d = 1 }' Cargo.toml && \
	cargo build --release && \
	git commit -m "v$$next" Cargo.toml Cargo.lock
