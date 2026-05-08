use std::ops::{Add, AddAssign, Deref};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Payload(Vec<u8>);

impl Payload {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn push(&mut self, b: u8) {
        self.0.push(b);
    }

    pub fn extend_from_slice(&mut self, s: &[u8]) {
        self.0.extend_from_slice(s);
    }
}

impl Deref for Payload {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for Payload {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl From<Vec<u8>> for Payload {
    fn from(v: Vec<u8>) -> Self {
        Self(v)
    }
}

impl From<&[u8]> for Payload {
    fn from(s: &[u8]) -> Self {
        Self(s.to_vec())
    }
}

impl<const N: usize> From<[u8; N]> for Payload {
    fn from(s: [u8; N]) -> Self {
        Self(s.to_vec())
    }
}

impl<const N: usize> From<&[u8; N]> for Payload {
    fn from(s: &[u8; N]) -> Self {
        Self(s.to_vec())
    }
}

impl From<&str> for Payload {
    fn from(s: &str) -> Self {
        Self(s.as_bytes().to_vec())
    }
}

impl From<String> for Payload {
    fn from(s: String) -> Self {
        Self(s.into_bytes())
    }
}

impl From<Payload> for Vec<u8> {
    fn from(p: Payload) -> Self {
        p.0
    }
}

impl AddAssign<Payload> for Payload {
    fn add_assign(&mut self, rhs: Payload) {
        self.0.extend_from_slice(&rhs.0);
    }
}

impl AddAssign<&Payload> for Payload {
    fn add_assign(&mut self, rhs: &Payload) {
        self.0.extend_from_slice(&rhs.0);
    }
}

impl AddAssign<&[u8]> for Payload {
    fn add_assign(&mut self, rhs: &[u8]) {
        self.0.extend_from_slice(rhs);
    }
}

impl AddAssign<Vec<u8>> for Payload {
    fn add_assign(&mut self, rhs: Vec<u8>) {
        self.0.extend(rhs);
    }
}

impl Add<Payload> for Payload {
    type Output = Payload;
    fn add(mut self, rhs: Payload) -> Payload {
        self += rhs;
        self
    }
}

impl Add<&Payload> for Payload {
    type Output = Payload;
    fn add(mut self, rhs: &Payload) -> Payload {
        self += rhs;
        self
    }
}

impl Add<&Payload> for &Payload {
    type Output = Payload;
    fn add(self, rhs: &Payload) -> Payload {
        let mut out = self.clone();
        out += rhs;
        out
    }
}

pub fn p8(v: u8) -> Payload {
    Payload(vec![v])
}

pub fn p16(v: u16) -> Payload {
    Payload(v.to_le_bytes().to_vec())
}

pub fn p32(v: u32) -> Payload {
    Payload(v.to_le_bytes().to_vec())
}

pub fn p64(v: u64) -> Payload {
    Payload(v.to_le_bytes().to_vec())
}

pub fn p16_be(v: u16) -> Payload {
    Payload(v.to_be_bytes().to_vec())
}

pub fn p32_be(v: u32) -> Payload {
    Payload(v.to_be_bytes().to_vec())
}

pub fn p64_be(v: u64) -> Payload {
    Payload(v.to_be_bytes().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_le_byte_order() {
        assert_eq!(p8(0xab).as_bytes(), &[0xab]);
        assert_eq!(p16(0x1234).as_bytes(), &[0x34, 0x12]);
        assert_eq!(p32(0xdeadbeef).as_bytes(), &[0xef, 0xbe, 0xad, 0xde]);
        assert_eq!(
            p64(0x0123_4567_89ab_cdef).as_bytes(),
            &[0xef, 0xcd, 0xab, 0x89, 0x67, 0x45, 0x23, 0x01]
        );
    }

    #[test]
    fn pack_be_byte_order() {
        assert_eq!(p16_be(0x1234).as_bytes(), &[0x12, 0x34]);
        assert_eq!(p32_be(0xdeadbeef).as_bytes(), &[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(
            p64_be(0x0123_4567_89ab_cdef).as_bytes(),
            &[0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]
        );
    }

    #[test]
    fn pack_edge_values() {
        assert_eq!(p32(0).as_bytes(), &[0, 0, 0, 0]);
        assert_eq!(p32(u32::MAX).as_bytes(), &[0xff, 0xff, 0xff, 0xff]);
        assert_eq!(p64(u64::MAX).as_bytes(), &[0xff; 8]);
    }

    #[test]
    fn from_conversions() {
        assert_eq!(Payload::from(b"hi".to_vec()).as_bytes(), b"hi");
        assert_eq!(Payload::from(b"hi" as &[u8]).as_bytes(), b"hi");
        assert_eq!(Payload::from(*b"hi").as_bytes(), b"hi");
        assert_eq!(Payload::from(b"hi").as_bytes(), b"hi");
        assert_eq!(Payload::from("hi").as_bytes(), b"hi");
        assert_eq!(Payload::from("hi".to_string()).as_bytes(), b"hi");
    }

    #[test]
    fn into_vec() {
        let v: Vec<u8> = Payload::from(b"hello".as_slice()).into();
        assert_eq!(v, b"hello");
    }

    #[test]
    fn add_assign_payload() {
        let mut p = Payload::from(b"foo".as_slice());
        p += Payload::from(b"bar".as_slice());
        assert_eq!(p.as_bytes(), b"foobar");
    }

    #[test]
    fn add_assign_ref_payload() {
        let mut p = Payload::from(b"foo".as_slice());
        let q = Payload::from(b"bar".as_slice());
        p += &q;
        assert_eq!(p.as_bytes(), b"foobar");
        assert_eq!(q.as_bytes(), b"bar");
    }

    #[test]
    fn add_assign_slice_and_vec() {
        let mut p = Payload::new();
        p += b"ab".as_slice();
        p += b"cd".to_vec();
        assert_eq!(p.as_bytes(), b"abcd");
    }

    #[test]
    fn add_op() {
        let a = Payload::from(b"foo".as_slice());
        let b = Payload::from(b"bar".as_slice());
        assert_eq!((a + b).as_bytes(), b"foobar");
    }

    #[test]
    fn add_op_with_ref_rhs() {
        let a = Payload::from(b"foo".as_slice());
        let b = Payload::from(b"bar".as_slice());
        assert_eq!((a + &b).as_bytes(), b"foobar");
        assert_eq!(b.as_bytes(), b"bar");
    }

    #[test]
    fn add_op_both_refs() {
        let a = Payload::from(b"foo".as_slice());
        let b = Payload::from(b"bar".as_slice());
        assert_eq!((&a + &b).as_bytes(), b"foobar");
        assert_eq!(a.as_bytes(), b"foo");
        assert_eq!(b.as_bytes(), b"bar");
    }

    #[test]
    fn add_assign_chain_with_pack() {
        let mut p = Payload::new();
        p += p8(0x41);
        p += p32(0x42);
        p += b"AB".as_slice();
        let bytes = p.into_bytes();
        assert_eq!(bytes.len(), 1 + 4 + 2);
        assert_eq!(bytes[0], 0x41);
        assert_eq!(&bytes[1..5], &[0x42, 0, 0, 0]);
        assert_eq!(&bytes[5..], b"AB");
    }

    #[test]
    fn deref_to_slice() {
        let p = Payload::from(b"hello".as_slice());
        let s: &[u8] = &p;
        assert_eq!(s, b"hello");
        assert_eq!(p.len(), 5);
    }

    #[test]
    fn as_ref_for_apis() {
        fn takes(b: impl AsRef<[u8]>) -> Vec<u8> {
            b.as_ref().to_vec()
        }
        assert_eq!(takes(Payload::from(b"hi".as_slice())), b"hi");
    }

    #[test]
    fn empty_default() {
        let p = Payload::default();
        assert!(p.is_empty());
        assert_eq!(p.len(), 0);
    }

    #[test]
    fn extend_from_slice() {
        let mut p = Payload::new();
        p.extend_from_slice(b"abc");
        p.extend_from_slice(b"def");
        assert_eq!(p.as_bytes(), b"abcdef");
    }

    #[test]
    fn push_byte() {
        let mut p = Payload::new();
        p.push(0x41);
        p.push(0x42);
        assert_eq!(p.as_bytes(), b"AB");
    }
}
