use std::fmt;

const MAX_DISPLAY_CHARS: usize = 200;

/// One argument of a server -> client function call (`OnConsoleMessage`, `OnSendToServer`, ...).
#[derive(Debug, Clone, PartialEq)]
pub enum Variant {
    Float(f32),
    String(String),
    Vec2(f32, f32),
    Vec3(f32, f32, f32),
    Unsigned(u32),
    Signed(i32),
}

impl Variant {
    fn type_id(&self) -> u8 {
        match self {
            Self::Float(_) => 1,
            Self::String(_) => 2,
            Self::Vec2(..) => 3,
            Self::Vec3(..) => 4,
            Self::Unsigned(_) => 5,
            Self::Signed(_) => 9,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Unsigned(v) => Some(*v as i64),
            Self::Signed(v) => Some(*v as i64),
            Self::Float(v) => Some(*v as i64),
            _ => None,
        }
    }
}

impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Float(v) => write!(f, "{v}"),
            // Some calls carry kilobytes of JSON/dialog markup; keep log lines readable.
            Self::String(v) if v.chars().count() > MAX_DISPLAY_CHARS => {
                let head: String = v.chars().take(MAX_DISPLAY_CHARS).collect();
                write!(f, "{head:?}... ({} chars)", v.chars().count())
            }
            Self::String(v) => write!(f, "{v:?}"),
            Self::Vec2(x, y) => write!(f, "({x}, {y})"),
            Self::Vec3(x, y, z) => write!(f, "({x}, {y}, {z})"),
            Self::Unsigned(v) => write!(f, "{v}u"),
            Self::Signed(v) => write!(f, "{v}"),
        }
    }
}

/// Wire format: `u8 count`, then per entry `u8 index, u8 type, value`.
/// Strings are `u32 len + bytes`; numbers are little-endian 32-bit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VariantList(pub Vec<Variant>);

impl VariantList {
    pub fn new(items: Vec<Variant>) -> Self {
        Self(items)
    }

    /// Returns `None` on truncated data or an unknown type id, since the
    /// remaining entries cannot be located without knowing its size.
    pub fn parse(data: &[u8]) -> Option<Self> {
        let mut r = Reader { data, pos: 0 };
        let count = r.u8()? as usize;
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            let _index = r.u8()?;
            let v = match r.u8()? {
                1 => Variant::Float(r.f32()?),
                2 => {
                    let len = r.u32()? as usize;
                    Variant::String(String::from_utf8_lossy(r.take(len)?).into_owned())
                }
                3 => Variant::Vec2(r.f32()?, r.f32()?),
                4 => Variant::Vec3(r.f32()?, r.f32()?, r.f32()?),
                5 => Variant::Unsigned(r.u32()?),
                9 => Variant::Signed(r.u32()? as i32),
                _ => return None,
            };
            items.push(v);
        }
        Some(Self(items))
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut b = vec![self.0.len() as u8];
        for (i, v) in self.0.iter().enumerate() {
            b.push(i as u8);
            b.push(v.type_id());
            match v {
                Variant::Float(x) => b.extend_from_slice(&x.to_le_bytes()),
                Variant::String(s) => {
                    b.extend_from_slice(&(s.len() as u32).to_le_bytes());
                    b.extend_from_slice(s.as_bytes());
                }
                Variant::Vec2(x, y) => {
                    b.extend_from_slice(&x.to_le_bytes());
                    b.extend_from_slice(&y.to_le_bytes());
                }
                Variant::Vec3(x, y, z) => {
                    b.extend_from_slice(&x.to_le_bytes());
                    b.extend_from_slice(&y.to_le_bytes());
                    b.extend_from_slice(&z.to_le_bytes());
                }
                Variant::Unsigned(x) => b.extend_from_slice(&x.to_le_bytes()),
                Variant::Signed(x) => b.extend_from_slice(&x.to_le_bytes()),
            }
        }
        b
    }

    pub fn get(&self, i: usize) -> Option<&Variant> {
        self.0.get(i)
    }

    /// The function name (first entry), e.g. `OnConsoleMessage`.
    pub fn function(&self) -> Option<&str> {
        self.get(0)?.as_str()
    }
}

impl fmt::Display for VariantList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, v) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "[{i}] {v}")?;
        }
        Ok(())
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f32(&mut self) -> Option<f32> {
        Some(f32::from_bits(self.u32()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let vl = VariantList::new(vec![
            Variant::String("OnSendToServer".into()),
            Variant::Signed(17040),
            Variant::Signed(4744778),
            Variant::Unsigned(7),
            Variant::String("213.179.209.175|0|02560A89".into()),
            Variant::Float(1.5),
            Variant::Vec2(1.0, 2.0),
            Variant::Vec3(1.0, 2.0, 3.0),
        ]);
        let bytes = vl.serialize();
        assert_eq!(VariantList::parse(&bytes).unwrap(), vl);
    }

    #[test]
    fn rejects_truncated() {
        let bytes = VariantList::new(vec![Variant::String("abc".into())]).serialize();
        assert!(VariantList::parse(&bytes[..bytes.len() - 1]).is_none());
    }
}
