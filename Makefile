.PHONY: validate fmt deps ast comments deny clippy test problem-details migrations fixtures hooks db sqlx-prepare tools

CARGO ?= cargo
TOOLS := $(CURDIR)/.tools
AST_GREP_VERSION := 0.51.2
CARGO_DENY_VERSION := 0.20.2
SQLX_CLI_VERSION := 0.9.0
export PATH := $(TOOLS)/bin:$(PATH)

validate: fmt deps ast comments deny clippy test problem-details migrations fixtures hooks db

fmt:
	$(CARGO) fmt --all --check

deps:
	checks/deps.sh

ast: tools
	ast-grep test --skip-snapshot-tests
	ast-grep scan

comments:
	files="$$($(CARGO) metadata --format-version 1 --no-deps | jq -r '.packages[].manifest_path' | xargs -n1 dirname | xargs -I{} find {} -name '*.rs' -not -path '*/target/*')"; \
	$(CARGO) run --quiet --release --locked -p guard -- $$files

deny: tools
	$(CARGO) deny check

clippy:
	$(CARGO) clippy --workspace --all-targets --locked -- -D warnings

test:
	$(CARGO) test --workspace --locked

problem-details:
	$(CARGO) run --quiet --locked -p xtask -- check-problem-details

migrations:
	$(CARGO) run --quiet --locked -p xtask -- check-migrations

fixtures: tools
	checks/lints/run.sh
	checks/ast/run.sh
	checks/deps-fixtures/run.sh
	checks/deny-fixtures/run.sh

hooks:
	python3 -m unittest discover -s scripts/tests -t . -q

db: tools
	checks/db/run.sh

sqlx-prepare: tools
	checks/db/run.sh prepare

tools:
	@ast-grep --version 2>/dev/null | grep -qx 'ast-grep $(AST_GREP_VERSION)' || \
		UV_TOOL_DIR=$(TOOLS)/uv UV_TOOL_BIN_DIR=$(TOOLS)/bin uv tool install --force ast-grep-cli==$(AST_GREP_VERSION)
	@cargo-deny --version 2>/dev/null | grep -qx 'cargo-deny $(CARGO_DENY_VERSION)' || \
		$(CARGO) install cargo-deny --version $(CARGO_DENY_VERSION) --locked --root $(TOOLS)
	@sqlx --version 2>/dev/null | grep -qx 'sqlx-cli $(SQLX_CLI_VERSION)' || \
		$(CARGO) install sqlx-cli --version $(SQLX_CLI_VERSION) --locked --no-default-features --features rustls,postgres --root $(TOOLS)
