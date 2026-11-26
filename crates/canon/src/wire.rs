//! Low-level CE primitives: minimal LEB128, length-prefixed bytes, names, levels.

use crate::types::{Level, Name, NamePart};
use alloc::{boxed::Box, string::String, vec::Vec};
use core::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Eof,
    BadTag(u8),
    NonMinimalVarint,
    Overflow,
    BadUtf8,
    BadRef,
    BadMagic,
    NonCanonical,
    TrailingBytes,
    TooDeep,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

/// Maximum nesting depth for levels.
const MAX_LEVEL_DEPTH: usize = 256;

#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn u8(&mut self, b: u8) {
        self.buf.push(b);
    }
    pub fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    pub fn uvar(&mut self, mut v: u64) {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.buf.push(byte);
                return;
            }
            self.buf.push(byte | 0x80);
        }
    }
    pub fn bytes(&mut self, b: &[u8]) {
        self.uvar(b.len() as u64);
        self.raw(b);
    }
    pub fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
    pub fn name(&mut self, n: &Name) {
        self.uvar(n.0.len() as u64);
        for p in &n.0 {
            match p {
                NamePart::Str(s) => {
                    self.u8(0);
                    self.str(s);
                }
                NamePart::Num(k) => {
                    self.u8(1);
                    self.uvar(*k);
                }
            }
        }
    }
    pub fn names(&mut self, ns: &[Name]) {
        self.uvar(ns.len() as u64);
        for n in ns {
            self.name(n);
        }
    }
    pub fn level(&mut self, l: &Level) {
        match l {
            Level::Zero => self.u8(0),
            Level::Succ(a) => {
                self.u8(1);
                self.level(a);
            }
            Level::Max(a, b) => {
                self.u8(2);
                self.level(a);
                self.level(b);
            }
            Level::IMax(a, b) => {
                self.u8(3);
                self.level(a);
                self.level(b);
            }
            Level::Param(n) => {
                self.u8(4);
                self.name(n);
            }
        }
    }
    pub fn finish(self) -> Vec<u8> {
        self.buf
    }
}

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    pub fn is_done(&self) -> bool {
        self.pos == self.buf.len()
    }
    pub fn finish(&self) -> Result<(), Error> {
        if self.is_done() {
            Ok(())
        } else {
            Err(Error::TrailingBytes)
        }
    }
    pub fn u8(&mut self) -> Result<u8, Error> {
        let b = *self.buf.get(self.pos).ok_or(Error::Eof)?;
        self.pos += 1;
        Ok(b)
    }
    pub fn raw(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Overflow)?;
        let s = self.buf.get(self.pos..end).ok_or(Error::Eof)?;
        self.pos = end;
        Ok(s)
    }
    pub fn array32(&mut self) -> Result<[u8; 32], Error> {
        let mut a = [0u8; 32];
        a.copy_from_slice(self.raw(32)?);
        Ok(a)
    }
    pub fn uvar(&mut self) -> Result<u64, Error> {
        let mut v: u64 = 0;
        let mut shift = 0u32;
        loop {
            let b = self.u8()?;
            let payload = (b & 0x7f) as u64;
            if shift == 63 && payload > 1 {
                return Err(Error::Overflow);
            }
            if shift > 63 {
                return Err(Error::Overflow);
            }
            v |= payload << shift;
            if b & 0x80 == 0 {
                // Minimal: the last group cannot be zero unless it is the only group.
                if b == 0 && shift > 0 {
                    return Err(Error::NonMinimalVarint);
                }
                return Ok(v);
            }
            shift += 7;
        }
    }
    /// Read a count and check it is plausible (each item needs at least one byte).
    pub fn count(&mut self) -> Result<usize, Error> {
        let n = self.uvar()?;
        if n > (self.buf.len() - self.pos) as u64 {
            return Err(Error::Eof);
        }
        Ok(n as usize)
    }
    pub fn bytes(&mut self) -> Result<&'a [u8], Error> {
        let n = self.uvar()?;
        let n = usize::try_from(n).map_err(|_| Error::Overflow)?;
        self.raw(n)
    }
    pub fn str(&mut self) -> Result<String, Error> {
        let b = self.bytes()?;
        core::str::from_utf8(b).map(String::from).map_err(|_| Error::BadUtf8)
    }
    pub fn name(&mut self) -> Result<Name, Error> {
        let n = self.count()?;
        let mut parts = Vec::with_capacity(n);
        for _ in 0..n {
            parts.push(match self.u8()? {
                0 => NamePart::Str(self.str()?),
                1 => NamePart::Num(self.uvar()?),
                t => return Err(Error::BadTag(t)),
            });
        }
        Ok(Name(parts))
    }
    pub fn names(&mut self) -> Result<Vec<Name>, Error> {
        let n = self.count()?;
        (0..n).map(|_| self.name()).collect()
    }
    pub fn level(&mut self) -> Result<Level, Error> {
        self.level_at(0)
    }
    fn level_at(&mut self, depth: usize) -> Result<Level, Error> {
        if depth > MAX_LEVEL_DEPTH {
            return Err(Error::TooDeep);
        }
        Ok(match self.u8()? {
            0 => Level::Zero,
            1 => Level::Succ(Box::new(self.level_at(depth + 1)?)),
            2 => Level::Max(
                Box::new(self.level_at(depth + 1)?),
                Box::new(self.level_at(depth + 1)?),
            ),
            3 => Level::IMax(
                Box::new(self.level_at(depth + 1)?),
                Box::new(self.level_at(depth + 1)?),
            ),
            4 => Level::Param(self.name()?),
            t => return Err(Error::BadTag(t)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uvar_roundtrip_and_minimality() {
        for v in [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX] {
            let mut w = Writer::new();
            w.uvar(v);
            let b = w.finish();
            let mut r = Reader::new(&b);
            assert_eq!(r.uvar().unwrap(), v);
            r.finish().unwrap();
        }
        // 0 encoded with a redundant continuation group.
        assert_eq!(Reader::new(&[0x80, 0x00]).uvar(), Err(Error::NonMinimalVarint));
        // 2^64 does not fit.
        let mut big = [0xffu8; 10];
        big[9] = 0x02;
        assert_eq!(Reader::new(&big).uvar(), Err(Error::Overflow));
    }
}
