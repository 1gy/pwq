//! Integration tests against real ELF binaries produced by GCC.
//! Guards against the circularity in the hand-crafted unit tests, where
//! the `build_elf` helper could share a spec misinterpretation with the
//! parser. Expected values were verified with `readelf` (see the fixture
//! README) and pin the parser against an external ground truth.

use pwq_elf::{Class, Elf, Endian};

const ELF_X86_64: &[u8] = include_bytes!("fixtures/test_x86_64.elf");
const ELF_I386: &[u8] = include_bytes!("fixtures/test_i386.elf");

#[test]
fn x86_64_header_matches_readelf() {
    let elf = Elf::from_bytes(ELF_X86_64).unwrap();
    assert_eq!(elf.class(), Class::Elf64);
    assert_eq!(elf.endian(), Endian::Little);
    assert_eq!(elf.machine(), 62); // EM_X86_64
    assert_eq!(elf.entry(), 0x401020);
}

#[test]
fn x86_64_symbol_addresses_match_readelf() {
    let elf = Elf::from_bytes(ELF_X86_64).unwrap();
    assert_eq!(elf.symbol("win"), Some(0x401106));
    assert_eq!(elf.symbol("target"), Some(0x401111));
    assert_eq!(elf.symbol("main"), Some(0x40111c));
}

#[test]
fn i386_header_matches_readelf() {
    let elf = Elf::from_bytes(ELF_I386).unwrap();
    assert_eq!(elf.class(), Class::Elf32);
    assert_eq!(elf.endian(), Endian::Little);
    assert_eq!(elf.machine(), 3); // EM_386
    assert_eq!(elf.entry(), 0x8049040);
}

#[test]
fn i386_symbol_addresses_match_readelf() {
    let elf = Elf::from_bytes(ELF_I386).unwrap();
    assert_eq!(elf.symbol("win"), Some(0x08049156));
    assert_eq!(elf.symbol("target"), Some(0x0804916a));
    assert_eq!(elf.symbol("main"), Some(0x0804917e));
}

#[test]
fn missing_symbol_returns_none() {
    let elf = Elf::from_bytes(ELF_X86_64).unwrap();
    assert_eq!(elf.symbol("__definitely_not_present__"), None);
}
