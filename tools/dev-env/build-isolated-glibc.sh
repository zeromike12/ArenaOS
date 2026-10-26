#!/usr/bin/env bash
# Reproducible isolated glibc 2.40 + pinned-QEMU wrappers (Phase 16 item 6).
#
# Purpose: the pinned Phase 15 patched QEMU 11.1.2 binary
# (docs/phase15/artifacts/phase15-patched-tcg-qualification.tar.gz,
# qemu-system-x86_64-patched sha256 e0ea2b1f...) requires GLIBC >= 2.38;
# the sandbox host has 2.36. The host system glibc is NEVER touched: this
# script builds glibc 2.40 from source into an isolated prefix and creates
# wrappers that invoke the pinned binary through the new ld.so.
#
# Tool chain (GitHub only; sandbox has no apt):
#   m4    tar-mirror/gnu-m4 tag v1.4.18 @ c8f51e6cbd0f5f75d9014222ce87caea626359ac
#         (an earlier session used f1a9aff6, no longer fetchable; the tag is
#         the same GNU M4 1.4.18 — only the *built glibc output* identity
#         matters and is recorded in docs/phase16/BACKEND-RECOVERY.md)
#   bison Distrotech/bison @ 48b6e4b0d5e63e4368bcc258360e2165322af6e0 (2.7.1)
#   gawk  gvlx/gawk @ a892293556960b0813098ede7da7a34774da7d3c (4.0.70)
#   glibc bminor/glibc release/2.40/master @ 9fe8576664d43b87ca19401fb6a975e217e47623
#
# Two classes of build fixes are required because these are git checkouts
# built with a modern toolchain:
#   * Timestamp freeze: git checkout times make generated autotools files
#     look stale, triggering aclocal/autoconf/autoheader regeneration by
#     tools that do not exist here. freeze() stamps sources older than
#     generated files, in dependency order.
#   * Old-gnulib vs modern glibc headers: SIGSTKSZ is no longer a
#     preprocessor constant, and _IO_* flag bits / _IO_ftrylockfile left
#     the public headers in glibc 2.28. Small patches restore them.
#
# set -e is deliberately NOT used mid-chain (bison/m4 doc targets fail
# after producing their binaries); every stage ends with a hard check.
# pipefail is required so "<tool> --version | head" checks cannot mask a
# missing or failing tool behind head's success.
set -x
set -o pipefail
WORK="$HOME/buildtools-src"
TOOLS="$HOME/buildtools"
export GLIBC_PREFIX="${GLIBC_PREFIX:-$HOME/glibc-2.40}"
PKG="$HOME/phase15-pkg"
WRAP="$HOME/phase16-backend"
export TZ=UTC
export PATH="$TOOLS/bin:$PATH"
export BISON_PKGDATADIR="$TOOLS/share/bison"
mkdir -p "$WORK" "$TOOLS/bin" "$WRAP"

freeze() { # freeze <dir>
    ( cd "$1" || exit 1
      find . -type f -exec touch -d "2020-01-01 00:00:00" {} +
      touch -d "2020-01-02 00:00:00" aclocal.m4 2>/dev/null
      touch -d "2020-01-03 00:00:00" configure 2>/dev/null
      find . -name "config.in.h" -o -name "*.hin" -o -name "config.h.in" -o -name "configh.in*" \
          | xargs -r touch -d "2020-01-07 00:00:00"
      find . -name "Makefile.in" | xargs -r touch -d "2020-01-08 00:00:00" )
}
clone() { # clone <url> <dir> <ref>
    [ -d "$WORK/$2/.git" ] || git clone "$1" "$WORK/$2" || exit 1
    git -C "$WORK/$2" checkout "$3" || exit 1
}
gnulib_patches() { # gnulib_patches <dir>
    ( cd "$1" || exit 1
      grep -l "_IO_ftrylockfile" lib/*.c 2>/dev/null | xargs -r sed -i \
        's/defined _IO_ftrylockfile/defined _IO_ftrylockfile || defined __GLIBC__/g'
      python3 - <<'PY'
p = "lib/stdio-impl.h"; s = open(p).read()
if "_IO_IN_BACKUP 0x100" not in s:
    add = """
#if defined __GLIBC__
# ifndef _IO_NO_READS
#  define _IO_NO_READS 4
# endif
# ifndef _IO_NO_WRITES
#  define _IO_NO_WRITES 8
# endif
# ifndef _IO_IN_BACKUP
#  define _IO_IN_BACKUP 0x100
# endif
# ifndef _IO_CURRENTLY_PUTTING
#  define _IO_CURRENTLY_PUTTING 0x800
# endif
# ifndef _IO_CURRENTLY_GETTING
#  define _IO_CURRENTLY_GETTING 0x400
# endif
#endif
"""
    s = s.replace("#include <errno.h>", "#include <errno.h>" + add, 1)
    open(p, "w").write(s)
try:
    p = "lib/c-stack.c"; s = open(p).read()
    old = "#ifndef SIGSTKSZ\n# define SIGSTKSZ 16384\n#elif HAVE_LIBSIGSEGV && SIGSTKSZ < 16384"
    if old in s:
        new = ("#if defined SIGSTKSZ && defined __GLIBC__\n# undef SIGSTKSZ\n"
               "# define SIGSTKSZ 32768\n#endif\n" + old)
        s = s.replace(old, new)
    open(p, "w").write(s)
except FileNotFoundError:
    pass
PY
      find lib -type f -exec touch -d "2020-01-01 00:00:00" {} + )
}

# --- m4 ----------------------------------------------------------------------
clone https://github.com/tar-mirror/gnu-m4 gnu-m4 c8f51e6cbd0f5f75d9014222ce87caea626359ac
if [ ! -x "$TOOLS/bin/m4" ]; then
    freeze "$WORK/gnu-m4"; gnulib_patches "$WORK/gnu-m4"; freeze "$WORK/gnu-m4"
    ( cd "$WORK/gnu-m4" && ./configure --prefix="$TOOLS" && { make -j4 || true; } )
    cp "$WORK/gnu-m4/src/m4" "$TOOLS/bin/m4"
fi
"$TOOLS/bin/m4" --version | head -1 || { echo "STAGE FAIL: m4"; exit 1; }

# --- bison (binary + data; full make exits 2 on docs — tolerated) -------------
clone https://github.com/Distrotech/bison bison 48b6e4b0d5e63e4368bcc258360e2165322af6e0
if [ ! -x "$TOOLS/bin/bison" ]; then
    freeze "$WORK/bison"; gnulib_patches "$WORK/bison"; freeze "$WORK/bison"
    ( cd "$WORK/bison" && M4="$TOOLS/bin/m4" ./configure --prefix="$TOOLS" && { make -j4 || true; } )
    mkdir -p "$TOOLS/share/bison"
    cp "$WORK/bison/src/bison" "$TOOLS/bin/bison"
    cp -r "$WORK/bison/data/"* "$TOOLS/share/bison/"
fi
ln -sf bison "$TOOLS/bin/yacc"
"$TOOLS/bin/bison" --version | head -1 || { echo "STAGE FAIL: bison"; exit 1; }

# --- gawk ---------------------------------------------------------------------
clone https://github.com/gvlx/gawk gawk a892293556960b0813098ede7da7a34774da7d3c
if [ ! -x "$TOOLS/bin/gawk" ]; then
    freeze "$WORK/gawk"
    ( cd "$WORK/gawk" && YACC="bison -y" ./configure --prefix="$TOOLS" && make -j4 ) || exit 1
    cp "$WORK/gawk/gawk" "$TOOLS/bin/gawk"
fi
"$TOOLS/bin/gawk" --version | head -1 || { echo "STAGE FAIL: gawk"; exit 1; }

# --- glibc 2.40 ---------------------------------------------------------------
clone https://github.com/bminor/glibc glibc 9fe8576664d43b87ca19401fb6a975e217e47623
mkdir -p "$HOME/glibc-build"
if [ ! -f "$GLIBC_PREFIX/lib/libc.so.6" ]; then
    ( cd "$HOME/glibc-build" \
      && AWK="$TOOLS/bin/gawk" "$WORK/glibc/configure" --prefix="$GLIBC_PREFIX" --disable-werror \
      && make -j4 && make install ) || { echo "STAGE FAIL: glibc"; exit 1; }
fi
"$GLIBC_PREFIX/lib/ld-linux-x86-64.so.2" --version | head -1 || exit 1

# --- pinned package + wrappers --------------------------------------------------
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
if [ ! -f "$PKG/backend/qemu-system-x86_64-patched" ]; then
    mkdir -p "$PKG"
    tar -xzf "$REPO/docs/phase15/artifacts/phase15-patched-tcg-qualification.tar.gz" -C "$PKG" || exit 1
fi
for kind in patched unpatched; do
    cat > "$WRAP/qemu-$kind" <<EOF
#!/bin/sh
exec "$GLIBC_PREFIX/lib/ld-linux-x86-64.so.2" \\
  --library-path "$GLIBC_PREFIX/lib:$PKG/backend/runtime/lib:/usr/lib/x86_64-linux-gnu" \\
  "$PKG/backend/qemu-system-x86_64-$kind" -accel tcg -L "$PKG/backend/pc-bios" "\$@"
EOF
    chmod +x "$WRAP/qemu-$kind"
done
sha256sum "$PKG/backend/qemu-system-x86_64-patched" "$GLIBC_PREFIX/lib/libc.so.6" \
          "$GLIBC_PREFIX/lib/ld-linux-x86-64.so.2"
"$WRAP/qemu-patched" --version | head -1 || exit 1
echo "ISOLATED-GLIBC SETUP COMPLETE"
