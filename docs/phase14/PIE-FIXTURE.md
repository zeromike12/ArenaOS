# Phase-14 PIE fixture inspection

Built with official Rust 1.97.0 (`rustc 1.97.0`, `rust-lld`) using
`userspace/phase14-pie/.cargo/config.toml` and `pie.ld`; inspected by GNU
readelf 2.44. The output is a genuine linker-produced ELF64 PIE, not a relabeled
`ET_EXEC`.

## ELF header

| Field | Value |
| --- | --- |
| Class / data / version | ELF64 / little-endian / current |
| OS/ABI / ABI version | System V / 0 |
| Type / machine | `ET_DYN` / `EM_X86_64` |
| Flags | 0 |
| Entry | `0x160` |
| ELF header / program header size | 64 / 56 bytes |
| Program header count | 5 |
| File size | 23,256 bytes |
| SHA-256 | `d7b0a8cf9c735c3898a867d824563f06b0d949df80fa1a9c96f9180a395fea2e` |

## Program headers

| Type | File offset | Virtual address | File size | Memory size | Flags | Alignment |
| --- | ---: | ---: | ---: | ---: | --- | ---: |
| `PT_LOAD` | `0x0000` | `0x0000` | `0x28ef` | `0x28ef` | R-X | `0x1000` |
| `PT_LOAD` | `0x3000` | `0x3000` | `0x621` | `0x621` | R-- | `0x1000` |
| `PT_LOAD` | `0x4000` | `0x4000` | `0x228` | `0x7000` | RW- | `0x1000` |
| `PT_DYNAMIC` | `0x4158` | `0x4158` | `0xd0` | `0xd0` | RW- | 8 |
| `PT_GNU_STACK` | `0` | `0` | 0 | 0 | RW- | 0 |

There is no `PT_INTERP`, TLS segment, executable stack, or W+X load segment.
The three `PT_LOAD` segments occupy 11 pages over an 11-page image span. The
initialized writable data, relative-relocation destinations, dynamic section,
BSS, and 16-KiB initial stack are all in the final RW/NX load segment.

## Dynamic records

The 13 records occupy 208 bytes. There are no dependency, PLT, text-relocation,
or TLS tags.

| Tag | Value |
| --- | ---: |
| `DT_FLAGS` | `DF_BIND_NOW` (`0x8`) |
| `DT_FLAGS_1` | `DF_1_NOW | DF_1_PIE` (`0x08000001`) |
| `DT_DEBUG` | 0 |
| `DT_RELA` | `0x3328` |
| `DT_RELASZ` | 720 |
| `DT_RELAENT` | 24 |
| `DT_RELACOUNT` | 30 |
| `DT_SYMTAB` | `0x35f8` |
| `DT_SYMENT` | 24 |
| `DT_STRTAB` | `0x3620` |
| `DT_STRSZ` | 1 |
| `DT_HASH` | `0x3610` |
| `DT_NULL` | 0 |

The dynamic symbol table contains exactly one all-zero null symbol. Its string
table is a single NUL byte. The SysV hash table is `(nbucket=1, nchain=1,
bucket[0]=0, chain[0]=0)`. All 30 relocations have symbol index zero. The
relocation table is 720 bytes, and the dynamic, symbol, string, hash, and
relocation metadata total 969 bytes.

## Relocation entries

Every record is `R_X86_64_RELATIVE` (type 8), symbol 0. Values are shown as
`r_offset <- r_addend`:

```text
0x4000 <- 0x311c    0x4018 <- 0x311c    0x4030 <- 0x311c
0x4048 <- 0x311c    0x4060 <- 0x311c    0x4078 <- 0x311c
0x4090 <- 0x033c    0x40a0 <- 0x032c    0x40a8 <- 0x0160
0x40b0 <- 0x0c71    0x40b8 <- 0x0ce8    0x40c0 <- 0x0c12
0x40c8 <- 0x0d1c    0x40d0 <- 0x6008    0x40d8 <- 0x106f
0x40e0 <- 0x0b40    0x40e8 <- 0x5008    0x40f0 <- 0x28c0
0x40f8 <- 0x0bbf    0x4100 <- 0x13b6    0x4108 <- 0x0b94
0x4110 <- 0x2680    0x4118 <- 0x2310    0x4120 <- 0x1ed7
0x4128 <- 0x2630    0x4130 <- 0x23e0    0x4138 <- 0x1f20
0x4140 <- 0x2520    0x4148 <- 0x1bc0    0x4150 <- 0x1f50
```

Destinations are unique, 8-byte aligned, and wholly inside writable non-X
memory. Each addend resolves inside a load segment. The application reads the
initialized and zero-initialized values, verifies the read-only constant, calls
the relocated function pointer, verifies its pointer against the relocated
function address, checks Startup ABI v2 entry/base, prints both addresses and
exits with status 42.
