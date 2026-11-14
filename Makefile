.PHONY: *

test:
	cargo insta test --unreferenced reject
	cargo bench --no-run
	cargo clippy -- -Dwarnings
	cargo fmt --check

test-depend:
	cargo install cargo-insta || true

bump-version:
	next=$$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1 | awk -F. '{print $$1"."$$2"."$$3+1}') && \
	perl -pi -e 'if (!$$d && s/^version = "[^"]+"/version = "'$$next'"/) { $$d = 1 }' Cargo.toml && \
	cargo build --release && \
	git commit -m "v$$next" Cargo.toml Cargo.lock

DELTA ?= /Users/dan/worktrees/delta/gitu/delta

# Benches commits not yet in benches/startup/results, then opens the chart.
bench-startup-chart:
	open $$(benches/startup/bench-history --delta $(DELTA) | tail -n1)
