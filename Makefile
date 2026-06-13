# Universal verb interface for the ironstate workspace.
#
# The Rust workspace lives under app/; every target drives cargo there so the
# repo root stays a thin control surface. `make help` lists everything; `make
# check` is the done-gate a human or an agent runs before calling work finished.

APP := app
CARGO := cargo
WASM_TARGET := wasm32-unknown-unknown
# Mirrors workspace.package.rust-version; the `msrv` target verifies it still builds.
MSRV := 1.96.0
# Wall-clock budget for a `make fuzz` run; CI overrides it.
FUZZ_SECONDS ?= 180
# Extra args for `make mutants` (CI passes `--in-diff` to scope to changed lines).
MUTANTS_ARGS ?=

# Run all cargo commands from inside the workspace directory.
CARGO_DIR := cd $(APP) &&

.DEFAULT_GOAL := help

.PHONY: help
help: ## List available targets
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) \
		| sort \
		| awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-16s\033[0m %s\n", $$1, $$2}'

.PHONY: build
build: ## Build the whole workspace
	$(CARGO_DIR) $(CARGO) build --workspace

.PHONY: test
test: ## Run the workspace test suite with all features
	$(CARGO_DIR) $(CARGO) test --workspace --all-features

.PHONY: msrv
msrv: ## Verify the workspace builds on the declared minimum Rust version
	$(CARGO_DIR) $(CARGO) +$(MSRV) check --workspace --all-features --locked

.PHONY: determinism-manifest
determinism-manifest: ## Print the determinism digests (from `make test`) as a sorted cross-target manifest
	@$(CARGO_DIR) find crates -path '*/ironstate-determinism/*.digest' \
		| sort \
		| while read -r f; do printf '%s %s\n' "$$(basename "$$f")" "$$(cat "$$f")"; done

.PHONY: fmt
fmt: ## Format the code
	$(CARGO_DIR) $(CARGO) fmt --all

.PHONY: fmt-check
fmt-check: ## Check formatting without writing changes
	$(CARGO_DIR) $(CARGO) fmt --all --check

.PHONY: clippy
clippy: ## Lint with clippy, warnings denied
	$(CARGO_DIR) $(CARGO) clippy --workspace --all-targets --all-features -- -D warnings

.PHONY: doc
doc: ## Build docs, warnings denied (every public item must be documented)
	$(CARGO_DIR) RUSTDOCFLAGS="-D warnings" $(CARGO) doc --workspace --all-features --no-deps

.PHONY: wasm
wasm: ## Build the determinism-sensitive crates for wasm32 (cross-target check)
	# No proptest on wasm: the determinism suite runs seeded, and proptest's RNG
	# pulls getrandom, which needs a wasm backend we deliberately avoid here.
	$(CARGO_DIR) $(CARGO) build -p ironstate --target $(WASM_TARGET) --no-default-features --features derive
	$(CARGO_DIR) $(CARGO) build -p ironstate-aggregate --target $(WASM_TARGET) --no-default-features --features audit

.PHONY: deny
deny: ## Supply-chain gate: licenses, advisories, duplicate majors
	$(CARGO_DIR) $(CARGO) deny check

.PHONY: fuzz
fuzz: ## Fuzz the versioned-restore decode path (needs nightly + cargo-fuzz); FUZZ_SECONDS to tune
	cd $(APP)/crates/ironstate && $(CARGO) +nightly fuzz run restore -- -max_total_time=$(FUZZ_SECONDS) -max_len=4096

.PHONY: mutants
mutants: ## Mutation-test the code (cargo-mutants); MUTANTS_ARGS='--in-diff <patch>' to scope to changes
	$(CARGO_DIR) $(CARGO) mutants $(MUTANTS_ARGS)

.PHONY: check
check: fmt-check clippy test ## The done-gate: formatting, lints, and tests

.PHONY: clean
clean: ## Remove build artifacts
	$(CARGO_DIR) $(CARGO) clean
