CC      := gcc
CFLAGS  := -O3 -Wall -Wextra -std=c11 -fPIC -DPIC -fstack-protector-strong -D_FORTIFY_SOURCE=2 -Iinclude $(shell pkg-config --cflags dbus-1 sbc 2>/dev/null)
LDFLAGS := -pthread -lm -Wl,-z,relro,-z,now
BT_LIBS := $(shell pkg-config --libs dbus-1 sbc 2>/dev/null || echo "-ldbus-1 -lsbc") -ldl
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

USE_RUST_WIRE ?= auto
CARGO := $(shell command -v cargo 2>/dev/null || ([ -x "$(HOME)/.cargo/bin/cargo" ] && echo "$(HOME)/.cargo/bin/cargo"))
ifeq ($(USE_RUST_WIRE),auto)
  ifeq ($(CARGO),)
    USE_RUST_WIRE_EFF := 0
  else
    USE_RUST_WIRE_EFF := 1
  endif
else
  USE_RUST_WIRE_EFF := $(USE_RUST_WIRE)
endif

WIRE_LIB := $(BUILD_DIR)/libsalsa_wire.a
RUST_SRCS := $(wildcard rust/salsa_net/src/*.rs) rust/salsa_net/Cargo.toml

ifeq ($(USE_RUST_WIRE_EFF),1)
AUDIODG_NET_SRCS :=
else
AUDIODG_NET_SRCS := src/lwasapi_pa_server.c src/lwasapi_pw_broker.c
endif

OPT_PA32 :=
ifneq ($(HOST_LIBPULSE32),)
ifeq ($(HAS_M32),1)
OPT_PA32 := $(PA_RECONNECT32_DIR)/libpulse.so.0
endif
endif

.PHONY: all clean test rust-test fuzz-diff test-sanitized installer dist

all: $(WIRE_LIB) \
     $(BUILD_DIR)/lwasapi-audiodg \
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
     $(BUILD_DIR)/lwasapi-bench

$(BUILD_DIR):
	mkdir -p $(BUILD_DIR)

$(SHIM_DIR):
	mkdir -p $(SHIM_DIR)

$(PA_RECONNECT_DIR):
	mkdir -p $(PA_RECONNECT_DIR)

$(PA_RECONNECT32_DIR):
	mkdir -p $(PA_RECONNECT32_DIR)

ifeq ($(USE_RUST_WIRE_EFF),1)
$(WIRE_LIB): $(RUST_SRCS) | $(BUILD_DIR)
	PATH="$(HOME)/.cargo/bin:$$PATH" $(CARGO) build --manifest-path rust/salsa_net/Cargo.toml --release --offline --target-dir $(BUILD_DIR)/rust_target
	cp $(BUILD_DIR)/rust_target/release/libsalsa_wire.a $@
else
$(BUILD_DIR)/salsa_wire_c.o: src/salsa_wire.c include/salsa_wire.h include/lwasapi.h | $(BUILD_DIR)
	$(CC) $(CFLAGS) -c src/salsa_wire.c -o $@

$(WIRE_LIB): $(BUILD_DIR)/salsa_wire_c.o | $(BUILD_DIR)
	$(AR) rcs $@ $(BUILD_DIR)/salsa_wire_c.o
endif

$(BUILD_DIR)/lwasapi-audiodg: src/lwasapi_audiodg.c src/lwasapi_bluez.c src/lwasapi_hw_analog.c src/lwasapi_midi.c src/lwasapi_debug.c $(AUDIODG_NET_SRCS) $(WIRE_LIB) include/lwasapi.h include/lwasapi_bluez.h include/lwasapi_hw_analog.h include/lwasapi_midi.h include/lwasapi_pa_server.h include/lwasapi_pw_broker.h include/lwasapi_debug.h include/salsa_wire.h | $(BUILD_DIR)
	$(CC) $(CFLAGS) src/lwasapi_audiodg.c src/lwasapi_bluez.c src/lwasapi_hw_analog.c src/lwasapi_midi.c src/lwasapi_debug.c $(AUDIODG_NET_SRCS) $(WIRE_LIB) -o $@ $(LDFLAGS) $(BT_LIBS)

$(BUILD_DIR)/liblwasapi.so: src/liblwasapi.c include/lwasapi.h include/lwasapi_debug.h | $(BUILD_DIR)
	$(CC) $(CFLAGS) -shared -Wl,-soname,liblwasapi.so src/liblwasapi.c -o $@ $(LDFLAGS)

$(BUILD_DIR)/libasound_module_pcm_lwasapi.so: src/alsa_plugin_lwasapi.c $(BUILD_DIR)/liblwasapi.so | $(BUILD_DIR)
	$(CC) $(CFLAGS) -shared -Wl,-soname,libasound_module_pcm_lwasapi.so \
		-Wl,-rpath,'$$ORIGIN' \
		src/alsa_plugin_lwasapi.c -L$(BUILD_DIR) -llwasapi $(HOST_LIBASOUND) -o $@ $(LDFLAGS)

$(BUILD_DIR)/libsalsa_pa_reconnect.so: src/salsa_pa_reconnect.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) -shared -Wl,-soname,libsalsa_pa_reconnect.so src/salsa_pa_reconnect.c -lpulse -ldl -o $@ $(LDFLAGS)

$(BUILD_DIR)/salsa_pa_reconnect_proxy.o: src/salsa_pa_reconnect.c | $(BUILD_DIR)
	$(CC) $(CFLAGS) -DSALSA_PA_PROXY -DSALSA_HOST_LIBPULSE=\"$(HOST_LIBPULSE)\" -c src/salsa_pa_reconnect.c -o $@

$(BUILD_DIR)/salsa_pa_proxy_stubs.S: $(BUILD_DIR)/salsa_pa_reconnect_proxy.o scripts/gen_pa_proxy.py src/pulse.version | $(BUILD_DIR)
	python3 scripts/gen_pa_proxy.py "$(HOST_LIBPULSE)" $(BUILD_DIR)/salsa_pa_reconnect_proxy.o $@

$(PA_RECONNECT_DIR)/libpulse.so.0: $(BUILD_DIR)/salsa_pa_reconnect_proxy.o $(BUILD_DIR)/salsa_pa_proxy_stubs.S src/pulse.version | $(PA_RECONNECT_DIR)
	$(CC) $(CFLAGS) -shared -Wl,-soname,libpulse.so.0 \
		-Wl,--version-script=src/pulse.version \
		-Wl,-z,nodelete \
		$(BUILD_DIR)/salsa_pa_reconnect_proxy.o $(BUILD_DIR)/salsa_pa_proxy_stubs.S -ldl -o $@ $(LDFLAGS)
	ln -sf libpulse.so.0 $(PA_RECONNECT_DIR)/libpulse.so

ifneq ($(OPT_PA32),)
CFLAGS32 := -m32 -O3 -Wall -Wextra -std=c11 -fPIC -DPIC -fstack-protector-strong -D_FORTIFY_SOURCE=2 -Iinclude

$(BUILD_DIR)/salsa_pa_reconnect_proxy32.o: src/salsa_pa_reconnect.c | $(BUILD_DIR)
	$(CC) $(CFLAGS32) -DSALSA_PA_PROXY -DSALSA_HOST_LIBPULSE=\"$(HOST_LIBPULSE32)\" -c src/salsa_pa_reconnect.c -o $@

$(BUILD_DIR)/salsa_pa_proxy_stubs32.S: $(BUILD_DIR)/salsa_pa_reconnect_proxy32.o scripts/gen_pa_proxy.py src/pulse.version | $(BUILD_DIR)
	python3 scripts/gen_pa_proxy.py --arch i386 "$(HOST_LIBPULSE32)" $(BUILD_DIR)/salsa_pa_reconnect_proxy32.o $@

$(PA_RECONNECT32_DIR)/libpulse.so.0: $(BUILD_DIR)/salsa_pa_reconnect_proxy32.o $(BUILD_DIR)/salsa_pa_proxy_stubs32.S src/pulse.version | $(PA_RECONNECT32_DIR)
	$(CC) $(CFLAGS32) -shared -Wl,-soname,libpulse.so.0 \
		-Wl,--version-script=src/pulse.version \
		-Wl,-z,nodelete \
		$(BUILD_DIR)/salsa_pa_reconnect_proxy32.o $(BUILD_DIR)/salsa_pa_proxy_stubs32.S -ldl -o $@ $(LDFLAGS)
	ln -sf libpulse.so.0 $(PA_RECONNECT32_DIR)/libpulse.so
endif

$(SHIM_DIR)/libasound.so.2: src/shim_alsa.c $(BUILD_DIR)/liblwasapi.so src/alsa.version | $(SHIM_DIR)
	$(CC) $(CFLAGS) -shared -Wl,-soname,libasound.so.2 \
		-Wl,--version-script=src/alsa.version \
		-Wl,-rpath,'$$ORIGIN/..' \
		src/shim_alsa.c -L$(BUILD_DIR) -llwasapi -ldl -o $@ $(LDFLAGS)
	ln -sf libasound.so.2 $(SHIM_DIR)/libasound.so

$(SHIM_DIR)/libpulse-simple.so.0: src/shim_pulse.c $(BUILD_DIR)/liblwasapi.so src/pulse.version | $(SHIM_DIR)
	$(CC) $(CFLAGS) -shared -Wl,-soname,libpulse-simple.so.0 \
		-Wl,--version-script=src/pulse.version \
		-Wl,-rpath,'$$ORIGIN/..' \
		src/shim_pulse.c -L$(BUILD_DIR) -llwasapi -o $@ $(LDFLAGS)
	ln -sf libpulse-simple.so.0 $(SHIM_DIR)/libpulse-simple.so

$(SHIM_DIR)/libpulse.so.0: src/shim_pulse.c $(BUILD_DIR)/liblwasapi.so src/pulse.version | $(SHIM_DIR)
	$(CC) $(CFLAGS) -shared -Wl,-soname,libpulse.so.0 \
		-Wl,--version-script=src/pulse.version \
		-Wl,-rpath,'$$ORIGIN/..' \
		src/shim_pulse.c -L$(BUILD_DIR) -llwasapi -o $@ $(LDFLAGS)
	ln -sf libpulse.so.0 $(SHIM_DIR)/libpulse.so

$(SHIM_DIR)/libpulse-mainloop-glib.so.0: src/shim_pulse.c $(BUILD_DIR)/liblwasapi.so src/pulse.version | $(SHIM_DIR)
	$(CC) $(CFLAGS) -shared -Wl,-soname,libpulse-mainloop-glib.so.0 \
		-Wl,--version-script=src/pulse.version \
		-Wl,-rpath,'$$ORIGIN/..' \
		src/shim_pulse.c -L$(BUILD_DIR) -llwasapi -o $@ $(LDFLAGS)
	ln -sf libpulse-mainloop-glib.so.0 $(SHIM_DIR)/libpulse-mainloop-glib.so

$(SHIM_DIR)/libjack.so.0: src/shim_jack.c $(BUILD_DIR)/liblwasapi.so | $(SHIM_DIR)
	$(CC) $(CFLAGS) -shared -Wl,-soname,libjack.so.0 \
		-Wl,-rpath,'$$ORIGIN/..' \
		src/shim_jack.c -L$(BUILD_DIR) -llwasapi -o $@ $(LDFLAGS)
	ln -sf libjack.so.0 $(SHIM_DIR)/libjack.so

$(BUILD_DIR)/lwasapi-ctl: src/lwasapi_ctl.c src/lwasapi_bluez.c src/lwasapi_debug.c $(WIRE_LIB) include/lwasapi.h include/lwasapi_bluez.h include/lwasapi_debug.h include/salsa_wire.h | $(BUILD_DIR)
	$(CC) $(CFLAGS) src/lwasapi_ctl.c src/lwasapi_bluez.c src/lwasapi_debug.c $(WIRE_LIB) -o $@ $(LDFLAGS) $(BT_LIBS)

$(BUILD_DIR)/lwasapi-test: src/lwasapi_test.c $(BUILD_DIR)/liblwasapi.so | $(BUILD_DIR)
	$(CC) $(CFLAGS) -Wl,-rpath,'$$ORIGIN' src/lwasapi_test.c -L$(BUILD_DIR) -llwasapi -ldl -o $@ $(LDFLAGS)

$(BUILD_DIR)/lwasapi-bench: src/lwasapi_bench.c $(BUILD_DIR)/liblwasapi.so | $(BUILD_DIR)
	$(CC) $(CFLAGS) -Wl,-rpath,'$$ORIGIN' src/lwasapi_bench.c -L$(BUILD_DIR) -llwasapi -ldl -o $@ $(LDFLAGS)

rust-test:
	PATH="$(HOME)/.cargo/bin:$$PATH" $(CARGO) clippy --manifest-path rust/salsa_net/Cargo.toml --offline --all-targets -- -D warnings
	PATH="$(HOME)/.cargo/bin:$$PATH" $(CARGO) test --manifest-path rust/salsa_net/Cargo.toml --offline

fuzz-diff: $(WIRE_LIB) tests/fuzz/fuzz_wire_diff.c tests/fuzz/c_wire_rename.h src/salsa_wire.c | $(BUILD_DIR)
	clang -O1 -g -fsanitize=address,undefined -Iinclude -include tests/fuzz/c_wire_rename.h -c src/salsa_wire.c -o $(BUILD_DIR)/c_wire_ref_fuzz.o
	clang -O1 -g -fsanitize=fuzzer,address,undefined -Iinclude tests/fuzz/fuzz_wire_diff.c $(BUILD_DIR)/c_wire_ref_fuzz.o $(WIRE_LIB) -o $(BUILD_DIR)/fuzz_wire_diff -pthread -lm -ldl

test-sanitized: rust-test fuzz-diff
	$(BUILD_DIR)/fuzz_wire_diff -runs=250000

test: all
	python3 scripts/run_regression_tests.py

REV ?= $(strip $(shell cat VERSION 2>/dev/null || echo v6.8-beta))

installer: all
	@if [ "$(REV)" != "$$(cat VERSION 2>/dev/null || true)" ]; then printf '%s\n' "$(REV)" > VERSION; fi
	python3 -c 'import os, stat, sys, zipfile; \
	rev = sys.argv[1]; \
	tag = rev if rev.startswith("v") else f"v{rev}"; \
	out = f"salsa-lwasapi-installer ({tag}).zip"; tmp = out + ".tmp"; \
	skip_dirs = {"__pycache__", ".git", ".pytest_cache", "target", "rust_target"}; \
	skip_names = {"fuzz_wire_diff", "c_wire_ref_fuzz.o"}; \
	files = []; \
	[files.extend([os.path.relpath(os.path.join(r, n), ".") for n in sorted(f) if n not in skip_names and not n.endswith((".pyc", ".zip", ".tmp", ".a"))]) for r, d, f in os.walk(".") if (d.__setitem__(slice(None), sorted(x for x in d if x not in skip_dirs)) or True)]; \
	zf = zipfile.ZipFile(tmp, "w", compression=zipfile.ZIP_DEFLATED); \
	[zf.writestr((lambda zi: (setattr(zi, "create_system", 3), setattr(zi, "external_attr", (stat.S_IFLNK | 0o777) << 16), zi)[-1])(zipfile.ZipInfo("salsa-lwasapi/" + p)), os.readlink(p).encode("utf-8")) if os.path.islink(p) else zf.write(p, "salsa-lwasapi/" + p) for p in sorted(files)]; \
	zf.close(); os.replace(tmp, out); print(f"Built {out} ({len(files)} entries)")' "$(REV)"

dist: installer

clean:
	rm -rf $(BUILD_DIR) rust/salsa_net/target
