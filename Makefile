BIN    := mclean
PREFIX ?= $(HOME)/.local
CARGO  ?= $(shell command -v cargo 2>/dev/null || echo $(HOME)/.cargo/bin/cargo)
SRC    := $(shell find src rules -type f) Cargo.toml Cargo.lock

.PHONY: all test lint fmt install uninstall clean

all: $(BIN)

$(BIN): $(SRC)
	@if [ ! -x "$(CARGO)" ]; then \
		echo "✖ Rust (cargo) is not installed. Either:"; \
		echo "    brew install rust"; \
		echo "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"; \
		exit 1; \
	fi
	$(CARGO) build --release --locked
	@rm -f ./$(BIN) && cp target/release/$(BIN) ./$(BIN)
	@echo ""
	@echo "✔ Ready. Run: ./$(BIN)"

test:
	$(CARGO) test

lint:
	$(CARGO) fmt --check
	$(CARGO) clippy --all-targets -- -D warnings

fmt:
	$(CARGO) fmt

install: $(BIN)
	install -d $(PREFIX)/bin
	install -m 755 $(BIN) $(PREFIX)/bin/$(BIN)
	@echo "✔ Installed in $(PREFIX)/bin/$(BIN)"

uninstall:
	rm -f $(PREFIX)/bin/$(BIN)

clean:
	$(CARGO) clean
	rm -f $(BIN)
