# Test fixtures

Real ELF binaries built from a tiny C source ([`source.c`](source.c)) shipped in this repo.
Used by `tests/real_binary.rs` as a "spec ground truth" check — because the binaries are produced by an external toolchain (GCC), they catch spec misinterpretations that hand-crafted test inputs in `lib.rs` cannot.

## Rebuild

Requires `gcc` and (for `test_i386.elf`) `libc6-dev-i386`.

```sh
gcc -no-pie -O0 source.c -o test_x86_64.elf
gcc -m32 -no-pie -O0 source.c -o test_i386.elf
```

Flags:

- `-no-pie` — force `ET_EXEC` (modern GCC defaults to PIE / `ET_DYN`)
- `-O0` — keep `win` / `target` / `main` as separate uninlined symbols

## Expected values

These were verified with `readelf -h` and `readelf -s` and are encoded as assertions in `tests/real_binary.rs`.

### `test_x86_64.elf`

| Field       | Value                       |
| ----------- | --------------------------- |
| Class       | ELF64                       |
| Endian      | little                      |
| Machine     | 62 (EM_X86_64)              |
| Entry       | `0x401020`                  |
| `win`       | `0x401106`                  |
| `target`    | `0x401111`                  |
| `main`      | `0x40111c`                  |

### `test_i386.elf`

| Field       | Value                       |
| ----------- | --------------------------- |
| Class       | ELF32                       |
| Endian      | little                      |
| Machine     | 3 (EM_386)                  |
| Entry       | `0x8049040`                 |
| `win`       | `0x08049156`                |
| `target`    | `0x0804916a`                |
| `main`      | `0x0804917e`                |

The exact addresses depend on the GCC / linker version.
If you rebuild and the addresses change, update both this table and `tests/real_binary.rs`.
