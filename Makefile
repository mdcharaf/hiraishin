PLUGIN_DIR ?= $(HOME)/.config/zellij/plugins
WASM := target/wasm32-wasip1/release/hiraishin.wasm

.PHONY: build test install uninstall

build:
	cargo build --release

test:
	cargo test --target $$(rustc -vV | sed -n 's/^host: //p')

install: build
	mkdir -p $(PLUGIN_DIR)
	cp $(WASM) $(PLUGIN_DIR)/hiraishin.wasm
	@echo "Installed to $(PLUGIN_DIR)/hiraishin.wasm"

uninstall:
	rm -f $(PLUGIN_DIR)/hiraishin.wasm
