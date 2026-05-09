use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    BadMagic,
    UnsupportedClass(u8),
    UnsupportedEndian(u8),
    Truncated,
    InvalidSection(&'static str),
    InvalidString,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::BadMagic => write!(f, "not an ELF file (bad magic)"),
            Self::UnsupportedClass(c) => write!(f, "unsupported ELF class: {c}"),
            Self::UnsupportedEndian(e) => write!(f, "unsupported ELF endian: {e}"),
            Self::Truncated => write!(f, "ELF file truncated"),
            Self::InvalidSection(s) => write!(f, "invalid section: {s}"),
            Self::InvalidString => write!(f, "invalid string in ELF (non-UTF-8 or unterminated)"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Elf32,
    Elf64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endian {
    Little,
    Big,
}

#[derive(Debug)]
pub struct Elf {
    class: Class,
    endian: Endian,
    machine: u16,
    entry: u64,
    symbols: HashMap<String, u64>,
}

impl Elf {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::from_bytes(fs::read(path)?)
    }

    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        if bytes.len() < 4 || &bytes[0..4] != b"\x7fELF" {
            return Err(Error::BadMagic);
        }
        if bytes.len() < 16 {
            return Err(Error::Truncated);
        }
        let class = match bytes[4] {
            1 => Class::Elf32,
            2 => Class::Elf64,
            v => return Err(Error::UnsupportedClass(v)),
        };
        let endian = match bytes[5] {
            1 => Endian::Little,
            2 => Endian::Big,
            v => return Err(Error::UnsupportedEndian(v)),
        };

        let r = Reader {
            bytes: &bytes,
            endian,
        };
        let header = parse_elf_header(&r, class)?;
        let symbols = collect_symbols(&bytes, &header, class, endian)?;

        Ok(Self {
            class,
            endian,
            machine: header.machine,
            entry: header.entry,
            symbols,
        })
    }

    pub fn class(&self) -> Class {
        self.class
    }

    pub fn endian(&self) -> Endian {
        self.endian
    }

    pub fn machine(&self) -> u16 {
        self.machine
    }

    pub fn entry(&self) -> u64 {
        self.entry
    }

    pub fn symbol(&self, name: &str) -> Option<u64> {
        self.symbols.get(name).copied()
    }

    pub fn symbols(&self) -> impl ExactSizeIterator<Item = (&str, u64)> + '_ {
        self.symbols.iter().map(|(k, &v)| (k.as_str(), v))
    }
}

const SHT_SYMTAB: u32 = 2;
const SHT_DYNSYM: u32 = 11;
const SHN_UNDEF: u16 = 0;

struct ElfHeader {
    machine: u16,
    entry: u64,
    shoff: u64,
    shentsize: u16,
    shnum: u16,
}

struct SectionHeader {
    sh_type: u32,
    sh_offset: u64,
    sh_size: u64,
    sh_link: u32,
}

struct Reader<'a> {
    bytes: &'a [u8],
    endian: Endian,
}

impl<'a> Reader<'a> {
    fn read_n<const N: usize>(&self, off: usize) -> Result<[u8; N]> {
        self.bytes
            .get(off..)
            .and_then(|tail| tail.first_chunk::<N>())
            .copied()
            .ok_or(Error::Truncated)
    }

    fn u16(&self, off: usize) -> Result<u16> {
        let arr = self.read_n::<2>(off)?;
        Ok(match self.endian {
            Endian::Little => u16::from_le_bytes(arr),
            Endian::Big => u16::from_be_bytes(arr),
        })
    }

    fn u32(&self, off: usize) -> Result<u32> {
        let arr = self.read_n::<4>(off)?;
        Ok(match self.endian {
            Endian::Little => u32::from_le_bytes(arr),
            Endian::Big => u32::from_be_bytes(arr),
        })
    }

    fn u64(&self, off: usize) -> Result<u64> {
        let arr = self.read_n::<8>(off)?;
        Ok(match self.endian {
            Endian::Little => u64::from_le_bytes(arr),
            Endian::Big => u64::from_be_bytes(arr),
        })
    }
}

fn parse_elf_header(r: &Reader, class: Class) -> Result<ElfHeader> {
    let machine = r.u16(18)?;
    let (entry, shoff, fixed_off) = match class {
        Class::Elf32 => {
            let entry = r.u32(24)? as u64;
            let shoff = r.u32(32)? as u64;
            (entry, shoff, 40)
        }
        Class::Elf64 => {
            let entry = r.u64(24)?;
            let shoff = r.u64(40)?;
            (entry, shoff, 52)
        }
    };
    let shentsize = r.u16(fixed_off + 6)?; // e_shentsize
    let shnum = r.u16(fixed_off + 8)?; // e_shnum
    Ok(ElfHeader {
        machine,
        entry,
        shoff,
        shentsize,
        shnum,
    })
}

fn parse_section_header(r: &Reader, off: usize, class: Class) -> Result<SectionHeader> {
    let sh_type = r.u32(off + 4)?;
    let (sh_offset, sh_size, sh_link) = match class {
        Class::Elf32 => (
            r.u32(off + 16)? as u64,
            r.u32(off + 20)? as u64,
            r.u32(off + 24)?,
        ),
        Class::Elf64 => (r.u64(off + 24)?, r.u64(off + 32)?, r.u32(off + 40)?),
    };
    Ok(SectionHeader {
        sh_type,
        sh_offset,
        sh_size,
        sh_link,
    })
}

fn section_data<'a>(bytes: &'a [u8], section: &SectionHeader) -> Result<&'a [u8]> {
    let start = section.sh_offset as usize;
    let end = start
        .checked_add(section.sh_size as usize)
        .ok_or(Error::Truncated)?;
    bytes.get(start..end).ok_or(Error::Truncated)
}

fn collect_symbols(
    bytes: &[u8],
    header: &ElfHeader,
    class: Class,
    endian: Endian,
) -> Result<HashMap<String, u64>> {
    let r = Reader { bytes, endian };

    let mut sections = Vec::with_capacity(header.shnum as usize);
    for i in 0..header.shnum {
        let off = header
            .shoff
            .checked_add(
                (i as u64)
                    .checked_mul(header.shentsize as u64)
                    .ok_or(Error::Truncated)?,
            )
            .ok_or(Error::Truncated)?;
        sections.push(parse_section_header(&r, off as usize, class)?);
    }

    let mut symbols = HashMap::new();
    for section in &sections {
        if section.sh_type != SHT_SYMTAB && section.sh_type != SHT_DYNSYM {
            continue;
        }
        let strtab = sections
            .get(section.sh_link as usize)
            .ok_or(Error::InvalidSection("symbol table sh_link out of range"))?;
        let strtab_data = section_data(bytes, strtab)?;
        let symtab_data = section_data(bytes, section)?;
        parse_symbol_table(symtab_data, strtab_data, class, endian, &mut symbols)?;
    }
    Ok(symbols)
}

fn parse_symbol_table(
    symtab: &[u8],
    strtab: &[u8],
    class: Class,
    endian: Endian,
    out: &mut HashMap<String, u64>,
) -> Result<()> {
    let entry_size = match class {
        Class::Elf32 => 16,
        Class::Elf64 => 24,
    };
    if !symtab.len().is_multiple_of(entry_size) {
        return Err(Error::InvalidSection("symtab size not aligned"));
    }
    let r = Reader {
        bytes: symtab,
        endian,
    };

    let mut off = 0;
    while off < symtab.len() {
        // 32-bit and 64-bit Sym layouts differ; in 64-bit, value/size move
        // after info/other/shndx so the field offsets are not the same.
        let (name_off, value, shndx) = match class {
            Class::Elf32 => (r.u32(off)?, r.u32(off + 4)? as u64, r.u16(off + 14)?),
            Class::Elf64 => (r.u32(off)?, r.u64(off + 8)?, r.u16(off + 6)?),
        };
        off += entry_size;

        // Drop entries that don't refer to a defined location: the leading
        // NULL entry, and undefined externs (e.g. unresolved libc imports
        // in .dynsym). A defined symbol whose address happens to be 0 must
        // be kept, so we can't filter on `value == 0`.
        if shndx == SHN_UNDEF {
            continue;
        }
        let name = read_cstring(strtab, name_off as usize)?;
        if name.is_empty() {
            continue;
        }
        out.insert(name, value);
    }
    Ok(())
}

fn read_cstring(bytes: &[u8], offset: usize) -> Result<String> {
    let tail = bytes.get(offset..).ok_or(Error::Truncated)?;
    let end = tail
        .iter()
        .position(|&b| b == 0)
        .ok_or(Error::InvalidString)?;
    String::from_utf8(tail[..end].to_vec()).map_err(|_| Error::InvalidString)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_u16(v: u16, endian: Endian) -> [u8; 2] {
        match endian {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        }
    }

    fn pack_u32(v: u32, endian: Endian) -> [u8; 4] {
        match endian {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        }
    }

    fn pack_u64(v: u64, endian: Endian) -> [u8; 8] {
        match endian {
            Endian::Little => v.to_le_bytes(),
            Endian::Big => v.to_be_bytes(),
        }
    }

    /// Build a minimal but valid ELF (header + 4 sections: NULL/.shstrtab/.symtab/.strtab)
    /// with the given symbols. Used for round-trip parser tests.
    fn build_elf(
        class: Class,
        endian: Endian,
        entry: u64,
        machine: u16,
        symbols: &[(&str, u64)],
    ) -> Vec<u8> {
        let header_size = match class {
            Class::Elf32 => 52usize,
            Class::Elf64 => 64,
        };
        let shentsize = match class {
            Class::Elf32 => 40usize,
            Class::Elf64 => 64,
        };
        let sym_size = match class {
            Class::Elf32 => 16usize,
            Class::Elf64 => 24,
        };

        let mut shstrtab = vec![0u8];
        let shstrtab_name = shstrtab.len() as u32;
        shstrtab.extend_from_slice(b".shstrtab\0");
        let symtab_name = shstrtab.len() as u32;
        shstrtab.extend_from_slice(b".symtab\0");
        let strtab_name = shstrtab.len() as u32;
        shstrtab.extend_from_slice(b".strtab\0");

        let mut strtab = vec![0u8];
        let mut sym_name_offs = Vec::new();
        for (name, _) in symbols {
            sym_name_offs.push(strtab.len() as u32);
            strtab.extend_from_slice(name.as_bytes());
            strtab.push(0);
        }

        let mut symtab = vec![0u8; sym_size];
        for ((_, value), &name_off) in symbols.iter().zip(sym_name_offs.iter()) {
            let mut entry = vec![0u8; sym_size];
            match class {
                Class::Elf32 => {
                    entry[0..4].copy_from_slice(&pack_u32(name_off, endian));
                    entry[4..8].copy_from_slice(&pack_u32(*value as u32, endian));
                    entry[8..12].copy_from_slice(&pack_u32(0, endian));
                    entry[12] = 0x12; // STB_GLOBAL << 4 | STT_FUNC
                    entry[13] = 0;
                    entry[14..16].copy_from_slice(&pack_u16(1, endian));
                }
                Class::Elf64 => {
                    entry[0..4].copy_from_slice(&pack_u32(name_off, endian));
                    entry[4] = 0x12;
                    entry[5] = 0;
                    entry[6..8].copy_from_slice(&pack_u16(1, endian));
                    entry[8..16].copy_from_slice(&pack_u64(*value, endian));
                    entry[16..24].copy_from_slice(&pack_u64(0, endian));
                }
            }
            symtab.extend_from_slice(&entry);
        }

        let align = |off: u64| (8 - off % 8) % 8;
        let mut cur = header_size as u64;
        let shstrtab_off = cur;
        cur += shstrtab.len() as u64;
        let pad1 = align(cur);
        cur += pad1;
        let strtab_off = cur;
        cur += strtab.len() as u64;
        let pad2 = align(cur);
        cur += pad2;
        let symtab_off = cur;
        cur += symtab.len() as u64;
        let pad3 = align(cur);
        cur += pad3;
        let shoff = cur;

        let mut bytes = Vec::with_capacity(shoff as usize + 4 * shentsize);
        bytes.extend_from_slice(b"\x7fELF");
        bytes.push(match class {
            Class::Elf32 => 1,
            Class::Elf64 => 2,
        });
        bytes.push(match endian {
            Endian::Little => 1,
            Endian::Big => 2,
        });
        bytes.push(1);
        bytes.extend_from_slice(&[0u8; 9]);
        bytes.extend_from_slice(&pack_u16(2, endian));
        bytes.extend_from_slice(&pack_u16(machine, endian));
        bytes.extend_from_slice(&pack_u32(1, endian));
        match class {
            Class::Elf32 => {
                bytes.extend_from_slice(&pack_u32(entry as u32, endian));
                bytes.extend_from_slice(&pack_u32(0, endian));
                bytes.extend_from_slice(&pack_u32(shoff as u32, endian));
            }
            Class::Elf64 => {
                bytes.extend_from_slice(&pack_u64(entry, endian));
                bytes.extend_from_slice(&pack_u64(0, endian));
                bytes.extend_from_slice(&pack_u64(shoff, endian));
            }
        }
        bytes.extend_from_slice(&pack_u32(0, endian));
        bytes.extend_from_slice(&pack_u16(header_size as u16, endian));
        bytes.extend_from_slice(&pack_u16(0, endian));
        bytes.extend_from_slice(&pack_u16(0, endian));
        bytes.extend_from_slice(&pack_u16(shentsize as u16, endian));
        bytes.extend_from_slice(&pack_u16(4, endian));
        bytes.extend_from_slice(&pack_u16(1, endian));

        assert_eq!(bytes.len(), header_size);

        bytes.extend_from_slice(&shstrtab);
        bytes.extend(std::iter::repeat_n(0u8, pad1 as usize));
        bytes.extend_from_slice(&strtab);
        bytes.extend(std::iter::repeat_n(0u8, pad2 as usize));
        bytes.extend_from_slice(&symtab);
        bytes.extend(std::iter::repeat_n(0u8, pad3 as usize));

        let write_sh = |out: &mut Vec<u8>,
                        name: u32,
                        sh_type: u32,
                        offset: u64,
                        size: u64,
                        link: u32,
                        entsize: u64| {
            match class {
                Class::Elf32 => {
                    out.extend_from_slice(&pack_u32(name, endian));
                    out.extend_from_slice(&pack_u32(sh_type, endian));
                    out.extend_from_slice(&pack_u32(0, endian));
                    out.extend_from_slice(&pack_u32(0, endian));
                    out.extend_from_slice(&pack_u32(offset as u32, endian));
                    out.extend_from_slice(&pack_u32(size as u32, endian));
                    out.extend_from_slice(&pack_u32(link, endian));
                    out.extend_from_slice(&pack_u32(0, endian));
                    out.extend_from_slice(&pack_u32(0, endian));
                    out.extend_from_slice(&pack_u32(entsize as u32, endian));
                }
                Class::Elf64 => {
                    out.extend_from_slice(&pack_u32(name, endian));
                    out.extend_from_slice(&pack_u32(sh_type, endian));
                    out.extend_from_slice(&pack_u64(0, endian));
                    out.extend_from_slice(&pack_u64(0, endian));
                    out.extend_from_slice(&pack_u64(offset, endian));
                    out.extend_from_slice(&pack_u64(size, endian));
                    out.extend_from_slice(&pack_u32(link, endian));
                    out.extend_from_slice(&pack_u32(0, endian));
                    out.extend_from_slice(&pack_u64(0, endian));
                    out.extend_from_slice(&pack_u64(entsize, endian));
                }
            }
        };

        bytes.extend(std::iter::repeat_n(0u8, shentsize));
        write_sh(
            &mut bytes,
            shstrtab_name,
            3,
            shstrtab_off,
            shstrtab.len() as u64,
            0,
            0,
        );
        write_sh(
            &mut bytes,
            symtab_name,
            2,
            symtab_off,
            symtab.len() as u64,
            3,
            sym_size as u64,
        );
        write_sh(
            &mut bytes,
            strtab_name,
            3,
            strtab_off,
            strtab.len() as u64,
            0,
            0,
        );

        bytes
    }

    #[test]
    fn rejects_non_elf_bytes() {
        assert!(matches!(
            Elf::from_bytes(b"not an elf file".to_vec()),
            Err(Error::BadMagic)
        ));
    }

    #[test]
    fn rejects_truncated_header() {
        // Valid magic + e_ident bytes but cut short before the rest of the header.
        let mut bytes = vec![0u8; 8];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 1;
        assert!(matches!(Elf::from_bytes(bytes), Err(Error::Truncated)));
    }

    #[test]
    fn rejects_unsupported_class() {
        let mut bytes = vec![0u8; 16];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 5;
        bytes[5] = 1;
        assert!(matches!(
            Elf::from_bytes(bytes),
            Err(Error::UnsupportedClass(5))
        ));
    }

    #[test]
    fn rejects_unsupported_endian() {
        let mut bytes = vec![0u8; 16];
        bytes[0..4].copy_from_slice(b"\x7fELF");
        bytes[4] = 2;
        bytes[5] = 7;
        assert!(matches!(
            Elf::from_bytes(bytes),
            Err(Error::UnsupportedEndian(7))
        ));
    }

    #[test]
    fn parses_64bit_le_symbols() {
        let bytes = build_elf(
            Class::Elf64,
            Endian::Little,
            0x401000,
            62, // EM_X86_64
            &[("win", 0x4011f6), ("main", 0x40139a)],
        );
        let elf = Elf::from_bytes(bytes).unwrap();
        assert_eq!(elf.class(), Class::Elf64);
        assert_eq!(elf.endian(), Endian::Little);
        assert_eq!(elf.machine(), 62);
        assert_eq!(elf.entry(), 0x401000);
        assert_eq!(elf.symbol("win"), Some(0x4011f6));
        assert_eq!(elf.symbol("main"), Some(0x40139a));
        assert_eq!(elf.symbol("missing"), None);
        assert_eq!(elf.symbols().len(), 2);
    }

    #[test]
    fn parses_32bit_le_symbols() {
        let bytes = build_elf(
            Class::Elf32,
            Endian::Little,
            0x8048000,
            3, // EM_386
            &[("foo", 0x8048500)],
        );
        let elf = Elf::from_bytes(bytes).unwrap();
        assert_eq!(elf.class(), Class::Elf32);
        assert_eq!(elf.entry(), 0x8048000);
        assert_eq!(elf.symbol("foo"), Some(0x8048500));
    }

    #[test]
    fn parses_64bit_be_symbols() {
        let bytes = build_elf(
            Class::Elf64,
            Endian::Big,
            0x4000,
            0x16, // EM_S390
            &[("bar", 0x5000)],
        );
        let elf = Elf::from_bytes(bytes).unwrap();
        assert_eq!(elf.endian(), Endian::Big);
        assert_eq!(elf.symbol("bar"), Some(0x5000));
    }

    #[test]
    fn parses_32bit_be_symbols() {
        let bytes = build_elf(
            Class::Elf32,
            Endian::Big,
            0x10000,
            8, // EM_MIPS
            &[("baz", 0x10100)],
        );
        let elf = Elf::from_bytes(bytes).unwrap();
        assert_eq!(elf.class(), Class::Elf32);
        assert_eq!(elf.endian(), Endian::Big);
        assert_eq!(elf.symbol("baz"), Some(0x10100));
    }

    #[test]
    fn empty_symbol_table_yields_empty_map() {
        let bytes = build_elf(Class::Elf64, Endian::Little, 0, 62, &[]);
        let elf = Elf::from_bytes(bytes).unwrap();
        assert_eq!(elf.symbols().len(), 0);
    }

    #[test]
    fn symbol_with_zero_address_is_kept_when_defined() {
        // build_elf assigns shndx=1 to every symbol (defined). The parser
        // must not drop a defined symbol whose value happens to be 0.
        // Regression test for the previous `value == 0` heuristic.
        let bytes = build_elf(Class::Elf64, Endian::Little, 0, 62, &[("zero_addr", 0)]);
        let elf = Elf::from_bytes(bytes).unwrap();
        assert_eq!(elf.symbol("zero_addr"), Some(0));
    }

    #[test]
    fn cstring_unterminated_string_errors() {
        // Constructing this via build_elf is awkward — feed the helper directly.
        assert!(matches!(read_cstring(b"abc", 0), Err(Error::InvalidString)));
    }

    #[test]
    fn cstring_offset_out_of_range_errors() {
        assert!(matches!(read_cstring(b"abc\0", 99), Err(Error::Truncated)));
    }

    #[test]
    fn cstring_offset_at_end_of_table_errors() {
        // Boundary: offset == bytes.len() yields a 0-length tail with no NUL.
        assert!(matches!(
            read_cstring(b"abc\0", 4),
            Err(Error::InvalidString)
        ));
    }

    #[test]
    fn cstring_non_utf8_returns_invalid_string() {
        assert!(matches!(
            read_cstring(b"\xff\xfe\0", 0),
            Err(Error::InvalidString)
        ));
    }
}
