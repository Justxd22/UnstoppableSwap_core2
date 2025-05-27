# Show help for each of the justfile recipes
help:
	@just --list

# Build Monero C++ Codebase (currently disabled)
# build_monero_cpp:
#	just update_submodules
#	cd monero-sys/monero && make -j8 release

# Clean the Monero C++ Codebase
clean_monero_cpp:
	rm -rf monero-sys/monero/
	just update_submodules

# Builds the Rust bindings for Monero
monero_sys:
	just update_submodules
	cd monero-sys && cargo build

# Start the Tauri app
tauri:
	cd src-tauri && cargo tauri dev --no-watch -- -- --testnet

tauri-mainnet:
	cd src-tauri && cargo tauri dev --no-watch

# Install the GUI dependencies
gui_install:
	cd src-gui && yarn install

# Start the GUI Dev Server
web:
	cd src-gui && yarn dev

gui:
	just web & just tauri

gui-mainnet:
	just web & just tauri-mainnet

# Build the GUI
gui_build:
        cd src-gui && yarn build

# Run the Rust tests
tests:
        cargo nextest run

# Tests the Rust bindings for Monero
test_monero_sys:
        cd monero-sys && cargo nextest run

# Builds the ASB and Swap binaries
swap:
	cd swap && cargo build --bin asb --bin=swap

# Updates our submodules (currently only Monero C++ codebase)
update_submodules:
	cd monero-sys && git submodule update --init --recursive --force

# Run clippy checks
clippy:
	cargo clippy --workspace --all-targets --all-features -- -D warnings

# Generate the bindings for the Tauri API
bindings:
	cd src-gui && yarn run gen-bindings

# Format the code
fmt:
	dprint fmt

# Sometimes you have to prune the docker network to get the integration tests to work
docker-prune-network:
	docker network prune -f
