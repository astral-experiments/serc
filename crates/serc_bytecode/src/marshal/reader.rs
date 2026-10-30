//! The wire graph has no compiler-side ownership or interning provenance. Indexing
//! an arena preserves sharing without cloning subtrees or recursively dropping them.

use super::v6::{
    FLAG_REF, TYPE_ASCII, TYPE_ASCII_INTERNED, TYPE_BINARY_COMPLEX, TYPE_BINARY_FLOAT, TYPE_CODE,
    TYPE_ELLIPSIS, TYPE_FALSE, TYPE_FROZENDICT, TYPE_FROZENSET, TYPE_INT, TYPE_INTERNED, TYPE_LONG,
    TYPE_NONE, TYPE_REF, TYPE_SHORT_ASCII, TYPE_SHORT_ASCII_INTERNED, TYPE_SLICE, TYPE_SMALL_TUPLE,
    TYPE_STRING, TYPE_TRUE, TYPE_TUPLE, TYPE_UNICODE,
};
use crate::PythonVersion;

pub type Id = usize;

#[derive(Debug)]
pub enum Object<'a> {
    None,
    Bool(bool),
    Ellipsis,
    Int(i32),
    Long { negative: bool, digits: &'a [u8] },
    Float(u64),
    Complex(u64, u64),
    Bytes(&'a [u8]),
    Text { bytes: &'a [u8], interned: bool },
    Tuple(Vec<Id>),
    FrozenSet(Vec<Id>),
    FrozenDict(Vec<(Id, Id)>),
    Slice([Id; 3]),
    Code(Box<Code>),
}

#[derive(Debug)]
pub struct Code {
    pub args: usize,
    pub pos_only: usize,
    pub kw_only: usize,
    pub stack_size: usize,
    pub flags: u32,
    pub bytecode: Id,
    pub constants: Id,
    pub names: Id,
    pub locals: Id,
    pub local_kinds: Id,
    pub filename: Id,
    pub name: Id,
    pub qualname: Id,
    pub first_line: usize,
    pub locations: Id,
    pub exceptions: Id,
}

pub struct Module<'a> {
    pub version: PythonVersion,
    pub objects: Vec<Object<'a>>,
    pub root: Id,
}

impl<'a> Module<'a> {
    pub fn code(&self, id: Id) -> Result<&Code, String> {
        match self.objects.get(id) {
            Some(Object::Code(code)) => Ok(code),
            _ => Err("expected code object".into()),
        }
    }

    pub fn bytes(&self, id: Id) -> Result<&'a [u8], String> {
        match self.objects.get(id) {
            Some(Object::Bytes(bytes)) => Ok(bytes),
            _ => Err("expected byte string".into()),
        }
    }

    pub fn text(&self, id: Id) -> Result<&'a str, String> {
        match self.objects.get(id) {
            Some(Object::Text { bytes, .. }) => {
                std::str::from_utf8(bytes).map_err(|_| "unsupported surrogate in string".into())
            }
            _ => Err("expected string".into()),
        }
    }

    pub fn tuple(&self, id: Id) -> Result<&[Id], String> {
        match self.objects.get(id) {
            Some(Object::Tuple(items)) => Ok(items),
            _ => Err("expected tuple".into()),
        }
    }
}

pub fn read(bytes: &[u8]) -> Result<Module<'_>, String> {
    let header = bytes.get(..16).ok_or("truncated pyc header")?;
    let version = [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ]
    .into_iter()
    .find(|v| v.target().magic_number == header[..4])
    .ok_or("unknown pyc magic")?;
    if !matches!(&header[4..8], [0 | 1 | 3, 0, 0, 0]) {
        return Err(format!("{version}: invalid pyc flags at file offset 4"));
    }
    // Bound overall work even when reference chains expand a tiny input into a large graph.
    if bytes.len() > 64 << 20 {
        return Err(format!("{version}: pyc exceeds 64 MiB"));
    }
    let mut reader = Reader {
        remaining: &bytes[16..],
        version,
        objects: Vec::new(),
        refs: Vec::new(),
        budget: 1_000_000,
    };
    let root = reader.object(0).map_err(|message| {
        format!(
            "{version}: {message} at file offset {}",
            bytes.len() - reader.remaining.len()
        )
    })?;
    if !reader.remaining.is_empty() {
        return Err(format!(
            "{version}: trailing marshal bytes at file offset {}",
            bytes.len() - reader.remaining.len()
        ));
    }
    let module = Module {
        version,
        objects: reader.objects,
        root,
    };
    module
        .code(root)
        .map_err(|e| format!("{version}: {e} at file offset 16"))?;
    Ok(module)
}

struct Reader<'a> {
    remaining: &'a [u8],
    version: PythonVersion,
    objects: Vec<Object<'a>>,
    refs: Vec<Option<Id>>,
    budget: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let (bytes, rest) = self
            .remaining
            .split_at_checked(n)
            .ok_or("truncated marshal data")?;
        self.remaining = rest;
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn word(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn length(&mut self) -> Result<usize, String> {
        usize::try_from(self.word()?).map_err(|_| "negative length".into())
    }
    fn float(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn sequence(&mut self, count: usize, depth: usize) -> Result<Vec<Id>, String> {
        if count > self.remaining.len() || count > self.budget {
            return Err("marshal sequence exceeds input or node limit".into());
        }
        // Grow only as children are read. Nested, untrusted length fields can
        // otherwise reserve the same remaining budget at every nesting level.
        let mut items = Vec::new();
        for _ in 0..count {
            items.push(self.object(depth)?);
        }
        Ok(items)
    }

    fn object(&mut self, depth: usize) -> Result<Id, String> {
        if depth > 96 || self.budget == 0 {
            return Err("marshal nesting or node limit exceeded".into());
        }
        stacker::maybe_grow(256 * 1024, 2 * 1024 * 1024, || self.read_object(depth))
    }

    fn read_object(&mut self, depth: usize) -> Result<Id, String> {
        self.budget -= 1;
        let byte = self.byte()?;
        if byte == TYPE_REF {
            let index = self.length()?;
            return self
                .refs
                .get(index)
                .copied()
                .flatten()
                .ok_or_else(|| "invalid or cyclic marshal reference".into());
        }
        let slot = (byte & FLAG_REF != 0).then(|| {
            self.refs.push(None);
            self.refs.len() - 1
        });
        let tag = byte & !FLAG_REF;
        let child = depth + 1;
        let value = match tag {
            TYPE_NONE => Object::None,
            TYPE_TRUE | TYPE_FALSE => Object::Bool(tag == TYPE_TRUE),
            TYPE_ELLIPSIS => Object::Ellipsis,
            TYPE_INT => Object::Int(self.word()?),
            TYPE_LONG => {
                let size = self.word()?;
                let digits = self.take(
                    (size.unsigned_abs() as usize)
                        .checked_mul(2)
                        .ok_or("marshal long too large")?,
                )?;
                if digits.chunks_exact(2).any(|pair| pair[1] >= 128) {
                    return Err("invalid marshal long digit".into());
                }
                Object::Long {
                    negative: size < 0,
                    digits,
                }
            }
            TYPE_BINARY_FLOAT => Object::Float(self.float()?),
            TYPE_BINARY_COMPLEX => Object::Complex(self.float()?, self.float()?),
            TYPE_STRING
            | TYPE_UNICODE
            | TYPE_INTERNED
            | TYPE_ASCII
            | TYPE_ASCII_INTERNED
            | TYPE_SHORT_ASCII
            | TYPE_SHORT_ASCII_INTERNED => {
                let size = if matches!(tag, TYPE_SHORT_ASCII | TYPE_SHORT_ASCII_INTERNED) {
                    usize::from(self.byte()?)
                } else {
                    self.length()?
                };
                let bytes = self.take(size)?;
                if !matches!(tag, TYPE_STRING | TYPE_UNICODE | TYPE_INTERNED) && !bytes.is_ascii() {
                    return Err("non-ASCII byte in marshal ASCII string".into());
                }
                if tag == TYPE_STRING {
                    Object::Bytes(bytes)
                } else {
                    Object::Text {
                        bytes,
                        interned: matches!(
                            tag,
                            TYPE_INTERNED | TYPE_ASCII_INTERNED | TYPE_SHORT_ASCII_INTERNED
                        ),
                    }
                }
            }
            TYPE_SMALL_TUPLE | TYPE_TUPLE | TYPE_FROZENSET => {
                let size = if tag == TYPE_SMALL_TUPLE {
                    usize::from(self.byte()?)
                } else {
                    self.length()?
                };
                let items = self.sequence(size, child)?;
                if tag == TYPE_FROZENSET {
                    Object::FrozenSet(items)
                } else {
                    Object::Tuple(items)
                }
            }
            TYPE_SLICE if self.version >= PythonVersion::Py314 => Object::Slice([
                self.object(child)?,
                self.object(child)?,
                self.object(child)?,
            ]),
            TYPE_FROZENDICT if self.version == PythonVersion::Py315 => {
                let mut pairs = Vec::new();
                while self.remaining.first() != Some(&b'0') {
                    pairs.push((self.object(child)?, self.object(child)?));
                }
                self.byte()?;
                Object::FrozenDict(pairs)
            }
            TYPE_CODE => Object::Code(Box::new(Code {
                args: self.length()?,
                pos_only: self.length()?,
                kw_only: self.length()?,
                stack_size: self.length()?,
                flags: self.word()?.cast_unsigned(),
                bytecode: self.object(child)?,
                constants: self.object(child)?,
                names: self.object(child)?,
                locals: self.object(child)?,
                local_kinds: self.object(child)?,
                filename: self.object(child)?,
                name: self.object(child)?,
                qualname: self.object(child)?,
                first_line: self.length()?,
                locations: self.object(child)?,
                exceptions: self.object(child)?,
            })),
            _ => return Err(format!("unsupported marshal tag {tag:#x}")),
        };
        let id = self.objects.len();
        self.objects.push(value);
        if let Some(slot) = slot {
            self.refs[slot] = Some(id);
        }
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_items_keep_the_same_id() {
        let mut reader = Reader {
            remaining: &[
                TYPE_SMALL_TUPLE,
                2,
                TYPE_INT | FLAG_REF,
                7,
                0,
                0,
                0,
                TYPE_REF,
                0,
                0,
                0,
                0,
            ],
            version: PythonVersion::Py314,
            objects: Vec::new(),
            refs: Vec::new(),
            budget: 3,
        };
        let id = reader.object(0).unwrap();
        let Object::Tuple(items) = &reader.objects[id] else {
            panic!("expected tuple")
        };
        assert_eq!(items[0], items[1]);
    }
    #[test]
    fn rejects_invalid_references_and_resource_claims() {
        for (payload, reason) in [
            (&b"r\0\0\0\0"[..], "reference"),
            (b"(\xff\xff\xff\x7f", "limit"),
            (
                &[TYPE_SMALL_TUPLE | FLAG_REF, 1, TYPE_REF, 0, 0, 0, 0],
                "cyclic",
            ),
        ] {
            let mut pyc = crate::CPYTHON_TARGET.magic_number.to_vec();
            pyc.resize(16, 0);
            pyc.extend(payload);
            assert!(read(&pyc).err().unwrap().contains(reason));
        }
    }
}
