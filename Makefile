# SPDX-License-Identifier: LGPL-3.0-only
# Copyright (C) 2026 Linuxknows
#
# Salsa (LWASAPI) is free software: you can redistribute it and/or modify it
# under the terms of the GNU Lesser General Public License as published by
# the Free Software Foundation, version 3 of the License.

CC         := gcc
SO_LDFLAGS := -pthread -lm -Wl,-z,relro,-z,now -Wl,-z,noexecstack -Wl,--gc-sections -Wl,-S
HARDEN_CXXFLAGS := -O2 -D_FORTIFY_SOURCE=3 -fstack-protector-strong -Wformat -Wformat-security
HARDEN_LDFLAGS  := -Wl,-z,relro,-z,now -Wl,-z,noexecstack
HOST_LIBASOUND := $(shell ldconfig -p 2>/dev/null | awk '/libasound\.so\.2 /{print $$NF; exit}')
ifeq ($(HOST_LIBASOUND),)
HOST_LIBASOUND := $(shell for p in /usr/lib/libasound.so.2 /usr/lib64/libasound.so.2 /usr/lib/x86_64-linux-gnu/libasound.so.2 /usr/lib/aarch64-linux-gnu/libasound.so.2; do [ -e "$$p" ] && readlink -f "$$p" && break; done)
endif
ifeq ($(HOST_LIBASOUND),)
HOST_LIBASOUND := -lasound
endif

HOST_LIBPULSE := $(shell ldconfig -p 2>/dev/null | awk '/libpulse\.so\.0 / && $$0 !~ /lib32/ {print $$NF; exit}')
ifeq ($(HOST_LIBPULSE),)
HOST_LIBPULSE := $(shell for p in /usr/lib/libpulse.so.0 /usr/lib64/libpulse.so.0 /usr/lib/x86_64-linux-gnu/libpulse.so.0 /usr/lib/aarch64-linux-gnu/libpulse.so.0; do [ -e "$$p" ] && echo "$$p" && break; done)
endif

HOST_LIBPULSE32 := $(shell ldconfig -p 2>/dev/null | awk '/libpulse\.so\.0 / && ($$0 ~ /lib32/ || $$0 ~ /i386/ || $$0 !~ /x86-64|64bit|AArch64|aarch64/) {print $$NF; exit}')
ifeq ($(HOST_LIBPULSE32),)
HOST_LIBPULSE32 := $(shell for p in /usr/lib32/libpulse.so.0 /usr/lib/i386-linux-gnu/libpulse.so.0 /lib/i386-linux-gnu/libpulse.so.0; do [ -e "$$p" ] && echo "$$p" && break; done)
endif
HAS_M32 := $(shell printf 'int x;\n' | $(CC) -m32 -x c -c - -o /dev/null 2>/dev/null && echo 1 || echo 0)

BUILD_DIR          := build
SHIM_DIR           := $(BUILD_DIR)/shims
PA_RECONNECT_DIR   := $(BUILD_DIR)/pa_reconnect
PA_RECONNECT32_DIR := $(BUILD_DIR)/pa_reconnect32
RUST_TARGET_DIR    := $(BUILD_DIR)/rust_target
RUST_REL_DIR       := $(RUST_TARGET_DIR)/release

CARGO := $(shell command -v cargo 2>/dev/null || ([ -x "$(HOME)/.cargo/bin/cargo" ] && echo "$(HOME)/.cargo/bin/cargo"))
RUSTC := $(shell command -v rustc 2>/dev/null || ([ -x "$(HOME)/.cargo/bin/rustc" ] && echo "$(HOME)/.cargo/bin/rustc"))
RUST32_LIBDIR := $(shell PATH="$(HOME)/.cargo/bin:$$PATH" "$(RUSTC)" --target i686-unknown-linux-gnu --print target-libdir 2>/dev/null)
HAS_RUST32 := $(shell ls "$(RUST32_LIBDIR)"/libstd-*.rlib >/dev/null 2>&1 && echo 1 || echo 0)

WIRE_LIB := $(BUILD_DIR)/libsalsa_wire.a
RUST_SRCS := rust/Cargo.toml rust/Cargo.lock $(wildcard rust/*/Cargo.toml) $(wildcard rust/*/src/*.rs) $(wildcard rust/*/src/bin/*.rs)

OPT_PA32 :=
ifneq ($(HOST_LIBPULSE32),)
ifeq ($(HAS_M32),1)
ifeq ($(HAS_RUST32),1)
OPT_PA32 := $(PA_RECONNECT32_DIR)/libpulse.so.0
endif
endif
endif

.PHONY: all clean test rust-test fuzz-wire test-sanitized installer dist

all: $(WIRE_LIB) \
     $(BUILD_DIR)/lwasapi-audiodg \
     $(BUILD_DIR)/salsa-gen-pa-proxy \
     $(BUILD_DIR)/liblwasapi.so \
     $(BUILD_DIR)/libasound_module_pcm_lwasapi.so \
     $(BUILD_DIR)/libsalsa_pa_reconnect.so \
     $(PA_RECONNECT_DIR)/libpulse.so.0 \
     $(OPT_PA32) \
     $(SHIM_DIR)/libasound.so.2 \
     $(SHIM_DIR)/libpulse-simple.so.0 \
     $(SHIM_DIR)/libpulse.so.0 \
     $(SHIM_DIR)/libpulse-mainloop-glib.so.0 \
     $(SHIM_DIR)/libjack.so.0 \
     $(BUILD_DIR)/lwasapi-ctl \
     $(BUILD_DIR)/lwasapi-test \
     $(BUILD_DIR)/lwasapi-bench \
     $(BUILD_DIR)/lwasapi-run \
     $(BUILD_DIR)/lwasapi-autosync \
     $(BUILD_DIR)/salsa-installer \
     $(BUILD_DIR)/salsa-dist \
     $(BUILD_DIR)/salsa-debug-cli \
     $(BUILD_DIR)/salsa-regression \
     $(BUILD_DIR)/lwasapi-tray \
     $(BUILD_DIR)/salsa-debug \
     $(BUILD_DIR)/salsa-studio

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(SHIM_DIR):
	mkdir -p $(SHIM_DIR)

$(PA_RECONNECT_DIR):
	mkdir -p $(PA_RECONNECT_DIR)

$(PA_RECONNECT32_DIR):
	mkdir -p $(PA_RECONNECT32_DIR)

$(WIRE_LIB): $(RUST_SRCS) | $(BUILD_DIR)
	SALSA_HOST_LIBPULSE="$(HOST_LIBPULSE)" RUSTFLAGS="-C relocation-model=pic -C link-arg=-Wl,-z,relro,-z,now -C link-arg=-Wl,-z,noexecstack" PATH="$(HOME)/.cargo/bin:$$PATH" $(CARGO) build --manifest-path rust/Cargo.toml --workspace --release --offline --target-dir $(RUST_TARGET_DIR)
	if [ -f $@ ] && cmp -s $(RUST_REL_DIR)/libsalsa_wire.a $@; then :; else cp $(RUST_REL_DIR)/libsalsa_wire.a $@.tmp && mv -f $@.tmp $@; fi

$(BUILD_DIR)/lwasapi-audiodg: $(WIRE_LIB) | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/lwasapi-audiodg $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/salsa-gen-pa-proxy: $(WIRE_LIB) | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/salsa-gen-pa-proxy $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/fuzz_wire: $(WIRE_LIB) | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/fuzz_wire $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/liblwasapi.so: $(WIRE_LIB) src/lwasapi.version | $(BUILD_DIR)
	$(CC) -shared -Wl,-soname,liblwasapi.so \
		-Wl,--version-script=src/lwasapi.version \
		-Wl,--whole-archive $(RUST_REL_DIR)/liblwasapi.a -Wl,--no-whole-archive \
		-ldl -o $@.tmp $(SO_LDFLAGS) && mv -f $@.tmp $@

$(BUILD_DIR)/libasound_module_pcm_lwasapi.so: $(WIRE_LIB) $(BUILD_DIR)/liblwasapi.so src/alsa_plugin.version | $(BUILD_DIR)
	$(CC) -shared -Wl,-soname,libasound_module_pcm_lwasapi.so \
		-Wl,--version-script=src/alsa_plugin.version \
		-Wl,-rpath,'$$ORIGIN' \
		-Wl,--whole-archive $(RUST_REL_DIR)/libalsa_plugin_lwasapi.a -Wl,--no-whole-archive \
		-L$(BUILD_DIR) -llwasapi $(HOST_LIBASOUND) -ldl -o $@.tmp $(SO_LDFLAGS) && mv -f $@.tmp $@

$(BUILD_DIR)/libsalsa_pa_reconnect.so: $(WIRE_LIB) src/pa_reconnect.version | $(BUILD_DIR)
	$(CC) -shared -Wl,-soname,libsalsa_pa_reconnect.so \
		-Wl,--version-script=src/pa_reconnect.version \
		-Wl,--whole-archive $(RUST_REL_DIR)/libsalsa_pa_reconnect.a -Wl,--no-whole-archive \
		-Wl,--no-as-needed -lpulse -Wl,--as-needed -ldl -o $@.tmp $(SO_LDFLAGS) && mv -f $@.tmp $@

$(BUILD_DIR)/salsa_pa_proxy_stubs.S: $(WIRE_LIB) $(BUILD_DIR)/salsa-gen-pa-proxy src/pulse.version | $(BUILD_DIR)
	$(BUILD_DIR)/salsa-gen-pa-proxy "$(HOST_LIBPULSE)" $(RUST_REL_DIR)/libsalsa_pa_reconnect_proxy.a $@

$(PA_RECONNECT_DIR)/libpulse.so.0: $(WIRE_LIB) $(BUILD_DIR)/salsa_pa_proxy_stubs.S src/pulse.version | $(PA_RECONNECT_DIR)
	$(CC) -shared -Wl,-soname,libpulse.so.0 \
		-Wl,--version-script=src/pulse.version \
		-Wl,-z,nodelete \
		-Wl,--whole-archive $(RUST_REL_DIR)/libsalsa_pa_reconnect_proxy.a -Wl,--no-whole-archive \
		$(BUILD_DIR)/salsa_pa_proxy_stubs.S -ldl -o $@.tmp $(SO_LDFLAGS) && mv -f $@.tmp $@
	ln -sf libpulse.so.0 $(PA_RECONNECT_DIR)/libpulse.so.tmp && mv -f $(PA_RECONNECT_DIR)/libpulse.so.tmp $(PA_RECONNECT_DIR)/libpulse.so

ifneq ($(OPT_PA32),)
$(BUILD_DIR)/rust_target32/i686-unknown-linux-gnu/release/libsalsa_pa_reconnect_proxy.a: $(RUST_SRCS) | $(BUILD_DIR)
	SALSA_HOST_LIBPULSE="$(HOST_LIBPULSE32)" RUSTFLAGS="-C relocation-model=pic -C link-arg=-Wl,-z,relro,-z,now -C link-arg=-Wl,-z,noexecstack" PATH="$(HOME)/.cargo/bin:$$PATH" $(CARGO) build --manifest-path rust/Cargo.toml -p salsa_pa_reconnect_proxy --target i686-unknown-linux-gnu --release --offline --target-dir $(BUILD_DIR)/rust_target32

$(BUILD_DIR)/salsa_pa_proxy_stubs32.S: $(BUILD_DIR)/rust_target32/i686-unknown-linux-gnu/release/libsalsa_pa_reconnect_proxy.a $(BUILD_DIR)/salsa-gen-pa-proxy src/pulse.version | $(BUILD_DIR)
	$(BUILD_DIR)/salsa-gen-pa-proxy --arch i386 "$(HOST_LIBPULSE32)" $(BUILD_DIR)/rust_target32/i686-unknown-linux-gnu/release/libsalsa_pa_reconnect_proxy.a $@

$(PA_RECONNECT32_DIR)/libpulse.so.0: $(BUILD_DIR)/rust_target32/i686-unknown-linux-gnu/release/libsalsa_pa_reconnect_proxy.a $(BUILD_DIR)/salsa_pa_proxy_stubs32.S src/pulse.version | $(PA_RECONNECT32_DIR)
	$(CC) -m32 -shared -Wl,-soname,libpulse.so.0 \
		-Wl,--version-script=src/pulse.version \
		-Wl,-z,nodelete \
		-Wl,--whole-archive $(BUILD_DIR)/rust_target32/i686-unknown-linux-gnu/release/libsalsa_pa_reconnect_proxy.a -Wl,--no-whole-archive \
		$(BUILD_DIR)/salsa_pa_proxy_stubs32.S -ldl -o $@.tmp $(SO_LDFLAGS) && mv -f $@.tmp $@
	ln -sf libpulse.so.0 $(PA_RECONNECT32_DIR)/libpulse.so.tmp && mv -f $(PA_RECONNECT32_DIR)/libpulse.so.tmp $(PA_RECONNECT32_DIR)/libpulse.so
endif

$(SHIM_DIR)/libasound.so.2: $(WIRE_LIB) $(BUILD_DIR)/liblwasapi.so src/alsa.version | $(SHIM_DIR)
	$(CC) -shared -Wl,-soname,libasound.so.2 \
		-Wl,--version-script=src/alsa.version \
		-Wl,-rpath,'$$ORIGIN/..' \
		-Wl,--whole-archive $(RUST_REL_DIR)/libshim_alsa.a -Wl,--no-whole-archive \
		-L$(BUILD_DIR) -llwasapi -ldl -o $@.tmp $(SO_LDFLAGS) && mv -f $@.tmp $@
	ln -sf libasound.so.2 $(SHIM_DIR)/libasound.so.tmp && mv -f $(SHIM_DIR)/libasound.so.tmp $(SHIM_DIR)/libasound.so

$(SHIM_DIR)/libpulse-simple.so.0: $(WIRE_LIB) $(BUILD_DIR)/liblwasapi.so src/pulse.version | $(SHIM_DIR)
	$(CC) -shared -Wl,-soname,libpulse-simple.so.0 \
		-Wl,--version-script=src/pulse.version \
		-Wl,-rpath,'$$ORIGIN/..' \
		-Wl,--whole-archive $(RUST_REL_DIR)/libshim_pulse.a -Wl,--no-whole-archive \
		-L$(BUILD_DIR) -llwasapi -ldl -o $@.tmp $(SO_LDFLAGS) && mv -f $@.tmp $@
	ln -sf libpulse-simple.so.0 $(SHIM_DIR)/libpulse-simple.so.tmp && mv -f $(SHIM_DIR)/libpulse-simple.so.tmp $(SHIM_DIR)/libpulse-simple.so

$(SHIM_DIR)/libpulse.so.0: $(WIRE_LIB) $(BUILD_DIR)/liblwasapi.so src/pulse.version | $(SHIM_DIR)
	$(CC) -shared -Wl,-soname,libpulse.so.0 \
		-Wl,--version-script=src/pulse.version \
		-Wl,-rpath,'$$ORIGIN/..' \
		-Wl,--whole-archive $(RUST_REL_DIR)/libshim_pulse.a -Wl,--no-whole-archive \
		-L$(BUILD_DIR) -llwasapi -ldl -o $@.tmp $(SO_LDFLAGS) && mv -f $@.tmp $@
	ln -sf libpulse.so.0 $(SHIM_DIR)/libpulse.so.tmp && mv -f $(SHIM_DIR)/libpulse.so.tmp $(SHIM_DIR)/libpulse.so

$(SHIM_DIR)/libpulse-mainloop-glib.so.0: $(WIRE_LIB) $(BUILD_DIR)/liblwasapi.so src/pulse.version | $(SHIM_DIR)
	$(CC) -shared -Wl,-soname,libpulse-mainloop-glib.so.0 \
		-Wl,--version-script=src/pulse.version \
		-Wl,-rpath,'$$ORIGIN/..' \
		-Wl,--whole-archive $(RUST_REL_DIR)/libshim_pulse.a -Wl,--no-whole-archive \
		-L$(BUILD_DIR) -llwasapi -ldl -o $@.tmp $(SO_LDFLAGS) && mv -f $@.tmp $@
	ln -sf libpulse-mainloop-glib.so.0 $(SHIM_DIR)/libpulse-mainloop-glib.so.tmp && mv -f $(SHIM_DIR)/libpulse-mainloop-glib.so.tmp $(SHIM_DIR)/libpulse-mainloop-glib.so

$(SHIM_DIR)/libjack.so.0: $(WIRE_LIB) $(BUILD_DIR)/liblwasapi.so src/jack.version | $(SHIM_DIR)
	$(CC) -shared -Wl,-soname,libjack.so.0 \
		-Wl,--version-script=src/jack.version \
		-Wl,-rpath,'$$ORIGIN/..' \
		-Wl,--whole-archive $(RUST_REL_DIR)/libshim_jack.a -Wl,--no-whole-archive \
		-L$(BUILD_DIR) -llwasapi -ldl -o $@.tmp $(SO_LDFLAGS) && mv -f $@.tmp $@
	ln -sf libjack.so.0 $(SHIM_DIR)/libjack.so.tmp && mv -f $(SHIM_DIR)/libjack.so.tmp $(SHIM_DIR)/libjack.so

.PHONY: fuzz-wire

$(BUILD_DIR)/lwasapi-ctl: $(WIRE_LIB) | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/lwasapi-ctl $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/lwasapi-test: $(WIRE_LIB) $(BUILD_DIR)/liblwasapi.so $(BUILD_DIR)/libasound_module_pcm_lwasapi.so $(SHIM_DIR)/libasound.so.2 $(SHIM_DIR)/libpulse.so.0 $(SHIM_DIR)/libjack.so.0 | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/lwasapi-test $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/lwasapi-bench: $(WIRE_LIB) $(BUILD_DIR)/liblwasapi.so | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/lwasapi-bench $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/lwasapi-run: $(WIRE_LIB) | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/lwasapi-run $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/lwasapi-autosync: $(WIRE_LIB) | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/lwasapi-autosync $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/salsa-installer: $(WIRE_LIB) | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/salsa-installer $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/salsa-dist: $(WIRE_LIB) | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/salsa-dist $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/salsa-debug-cli: $(WIRE_LIB) $(BUILD_DIR)/liblwasapi.so | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/salsa-debug-cli $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/salsa-regression: $(WIRE_LIB) $(BUILD_DIR)/liblwasapi.so | $(BUILD_DIR)
	cp $(RUST_REL_DIR)/salsa-regression $@.tmp && mv -f $@.tmp $@

QT6_OBJ_DIR := $(BUILD_DIR)/qt6_obj
CXX ?= g++
QT6_MOC ?= $(shell pkg-config --variable=libexecdir Qt6Core 2>/dev/null || echo /usr/lib/qt6)/moc
QT6_CFLAGS := $(shell pkg-config --cflags Qt6Widgets Qt6Gui Qt6Core)
QT6_LIBS := $(shell pkg-config --libs Qt6Widgets Qt6Gui Qt6Core) -lgcc_s -lutil -lrt -lpthread -lm -ldl
QT6_TRAY_CFLAGS := $(shell pkg-config --cflags Qt6DBus)
QT6_TRAY_LIBS := $(shell pkg-config --libs Qt6DBus)
QT6_COMMON_HDRS := qt6/salsa_bridge.h qt6/salsa_common.hpp qt6/salsa_widgets.hpp
QT6_COMMON_OBJS := $(QT6_OBJ_DIR)/salsa_common.o $(QT6_OBJ_DIR)/moc_salsa_common.o
QT6_STUDIO_OBJS := $(QT6_COMMON_OBJS) $(QT6_OBJ_DIR)/salsa_widgets.o $(QT6_OBJ_DIR)/moc_salsa_widgets.o $(QT6_OBJ_DIR)/salsa_debugger_widget.o

$(QT6_OBJ_DIR):
	mkdir -p $(QT6_OBJ_DIR)

$(QT6_OBJ_DIR)/moc_salsa_common.cpp: qt6/salsa_common.hpp | $(QT6_OBJ_DIR)
	$(QT6_MOC) $< -o $@

$(QT6_OBJ_DIR)/moc_salsa_widgets.cpp: qt6/salsa_widgets.hpp | $(QT6_OBJ_DIR)
	$(QT6_MOC) $< -o $@

$(QT6_OBJ_DIR)/salsa_studio.moc: qt6/salsa_studio.cpp | $(QT6_OBJ_DIR)
	$(QT6_MOC) $< -o $@

$(QT6_OBJ_DIR)/lwasapi_tray.moc: qt6/lwasapi_tray.cpp | $(QT6_OBJ_DIR)
	$(QT6_MOC) $< -o $@

$(QT6_OBJ_DIR)/salsa_debug.moc: qt6/salsa_debug.cpp | $(QT6_OBJ_DIR)
	$(QT6_MOC) $< -o $@

$(QT6_OBJ_DIR)/salsa_common.o: qt6/salsa_common.cpp $(QT6_COMMON_HDRS) | $(QT6_OBJ_DIR)
	$(CXX) -std=c++20 $(HARDEN_CXXFLAGS) -Wall -Wextra -fPIC -Iqt6 -I$(QT6_OBJ_DIR) $(QT6_CFLAGS) -c $< -o $@

$(QT6_OBJ_DIR)/moc_salsa_common.o: $(QT6_OBJ_DIR)/moc_salsa_common.cpp | $(QT6_OBJ_DIR)
	$(CXX) -std=c++20 $(HARDEN_CXXFLAGS) -fPIC -Iqt6 $(QT6_CFLAGS) -c $< -o $@

$(QT6_OBJ_DIR)/salsa_widgets.o: qt6/salsa_widgets.cpp $(QT6_COMMON_HDRS) | $(QT6_OBJ_DIR)
	$(CXX) -std=c++20 $(HARDEN_CXXFLAGS) -Wall -Wextra -fPIC -Iqt6 -I$(QT6_OBJ_DIR) $(QT6_CFLAGS) -c $< -o $@

$(QT6_OBJ_DIR)/moc_salsa_widgets.o: $(QT6_OBJ_DIR)/moc_salsa_widgets.cpp | $(QT6_OBJ_DIR)
	$(CXX) -std=c++20 $(HARDEN_CXXFLAGS) -fPIC -Iqt6 $(QT6_CFLAGS) -c $< -o $@

$(QT6_OBJ_DIR)/salsa_debugger_widget.o: qt6/salsa_debugger_widget.cpp $(QT6_COMMON_HDRS) | $(QT6_OBJ_DIR)
	$(CXX) -std=c++20 $(HARDEN_CXXFLAGS) -Wall -Wextra -fPIC -Iqt6 -I$(QT6_OBJ_DIR) $(QT6_CFLAGS) -c $< -o $@

$(QT6_OBJ_DIR)/salsa_studio.o: qt6/salsa_studio.cpp $(QT6_OBJ_DIR)/salsa_studio.moc $(QT6_COMMON_HDRS) | $(QT6_OBJ_DIR)
	$(CXX) -std=c++20 $(HARDEN_CXXFLAGS) -Wall -Wextra -fPIC -Iqt6 -I$(QT6_OBJ_DIR) $(QT6_CFLAGS) -c $< -o $@

$(QT6_OBJ_DIR)/lwasapi_tray.o: qt6/lwasapi_tray.cpp $(QT6_OBJ_DIR)/lwasapi_tray.moc $(QT6_COMMON_HDRS) | $(QT6_OBJ_DIR)
	$(CXX) -std=c++20 $(HARDEN_CXXFLAGS) -Wall -Wextra -fPIC -Iqt6 -I$(QT6_OBJ_DIR) $(QT6_CFLAGS) $(QT6_TRAY_CFLAGS) -c $< -o $@

$(QT6_OBJ_DIR)/salsa_debug.o: qt6/salsa_debug.cpp $(QT6_OBJ_DIR)/salsa_debug.moc $(QT6_COMMON_HDRS) | $(QT6_OBJ_DIR)
	$(CXX) -std=c++20 $(HARDEN_CXXFLAGS) -Wall -Wextra -fPIC -Iqt6 -I$(QT6_OBJ_DIR) $(QT6_CFLAGS) -c $< -o $@

$(BUILD_DIR)/lwasapi-tray: $(WIRE_LIB) $(QT6_COMMON_OBJS) $(QT6_OBJ_DIR)/lwasapi_tray.o | $(BUILD_DIR)
	$(CXX) $(QT6_COMMON_OBJS) $(QT6_OBJ_DIR)/lwasapi_tray.o $(RUST_REL_DIR)/libsalsa_ui.a $(QT6_LIBS) $(QT6_TRAY_LIBS) $(HARDEN_LDFLAGS) -Wl,-S -o $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/salsa-debug: $(WIRE_LIB) $(QT6_STUDIO_OBJS) $(QT6_OBJ_DIR)/salsa_debug.o | $(BUILD_DIR)
	$(CXX) $(QT6_STUDIO_OBJS) $(QT6_OBJ_DIR)/salsa_debug.o $(RUST_REL_DIR)/libsalsa_ui.a $(QT6_LIBS) $(HARDEN_LDFLAGS) -Wl,-S -o $@.tmp && mv -f $@.tmp $@

$(BUILD_DIR)/salsa-studio: $(WIRE_LIB) $(QT6_STUDIO_OBJS) $(QT6_OBJ_DIR)/salsa_studio.o | $(BUILD_DIR)
	$(CXX) $(QT6_STUDIO_OBJS) $(QT6_OBJ_DIR)/salsa_studio.o $(RUST_REL_DIR)/libsalsa_ui.a $(QT6_LIBS) $(HARDEN_LDFLAGS) -Wl,-S -o $@.tmp && mv -f $@.tmp $@

rust-test:
	PATH="$(HOME)/.cargo/bin:$$PATH" $(CARGO) clippy --manifest-path rust/Cargo.toml --workspace --offline --all-targets -- -D warnings
	PATH="$(HOME)/.cargo/bin:$$PATH" $(CARGO) test --manifest-path rust/Cargo.toml --workspace --offline

fuzz-wire: $(BUILD_DIR)/fuzz_wire

test-sanitized: rust-test fuzz-wire
	$(BUILD_DIR)/fuzz_wire -runs=250000

test: all
	$(BUILD_DIR)/salsa-regression

REV ?= $(strip $(shell cat VERSION 2>/dev/null || echo V1.21.12))

installer: all
	$(BUILD_DIR)/salsa-dist "$(REV)"

dist: installer

clean:
	rm -rf $(BUILD_DIR) rust/target rust/*/target
