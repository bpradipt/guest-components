# Copyright (c) 2024 Contributors to the image-rs project
# SPDX-License-Identifier: Apache-2.0

ARCH ?= $(shell uname -m)

# Features for the minimal static pull-debug binary (no encryption, no KBS)
# Minimum compilable features:
#   snapshot-overlayfs   – OverlayFS layer storage (needs root/CAP_SYS_ADMIN)
#   oci-client-rustls    – HTTPS OCI registry client (pure-Rust TLS, no OpenSSL)
#   signature-cosign-rustls – pulls in the "signature" and "futures" modules that
#                             builder.rs / stream/unpack.rs import unconditionally
PULL_DEBUG_FEATURES ?= pull-debug-bin,snapshot-overlayfs,oci-client-rustls,signature-cosign-rustls

# Static musl target (x86_64 or aarch64)
ifeq ($(ARCH),x86_64)
  MUSL_TARGET = x86_64-unknown-linux-musl
else ifeq ($(ARCH),aarch64)
  MUSL_TARGET = aarch64-unknown-linux-musl
else
  MUSL_TARGET = $(ARCH)-unknown-linux-musl
endif

.PHONY: pull-debug pull-debug-static install-musl-target clean help

## Build pull_debug against the host toolchain (dynamic linking, for local testing)
pull-debug:
	cargo build --bin pull_debug \
		--no-default-features \
		--features "$(PULL_DEBUG_FEATURES)"
	@echo "Binary at: ../target/debug/pull_debug"

## Build pull_debug as a fully static musl binary suitable for production deployment
pull-debug-static: install-musl-target
	cargo build --bin pull_debug \
		--release \
		--target $(MUSL_TARGET) \
		--no-default-features \
		--features "$(PULL_DEBUG_FEATURES)"
	@echo "Static binary at: ../target/$(MUSL_TARGET)/release/pull_debug"
	@file ../target/$(MUSL_TARGET)/release/pull_debug

## Add the musl Rust target if not already installed
install-musl-target:
	rustup target add $(MUSL_TARGET)

clean:
	cargo clean

help:
	@echo "Targets:"
	@echo "  pull-debug         Build pull_debug (dynamic, debug build) for local testing"
	@echo "  pull-debug-static  Build pull_debug as a static musl binary (release)"
	@echo "  install-musl-target Install the musl Rust target via rustup"
	@echo ""
	@echo "Variables:"
	@echo "  ARCH=$(ARCH)  (override to cross-compile, e.g. ARCH=aarch64)"
	@echo "  PULL_DEBUG_FEATURES=$(PULL_DEBUG_FEATURES)"
	@echo ""
	@echo "Usage after build:"
	@echo "  pull_debug --image docker.io/library/busybox:latest"
	@echo "  pull_debug --image myregistry.example.com/myimage:tag --auth user:pass"
	@echo "  pull_debug --help"
