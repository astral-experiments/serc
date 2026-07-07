use super::v5::{
    TYPE_ASCII, TYPE_ASCII_INTERNED, TYPE_BINARY_COMPLEX, TYPE_BINARY_FLOAT, TYPE_ELLIPSIS,
    TYPE_FALSE, TYPE_INT, TYPE_INTERNED, TYPE_LONG, TYPE_NONE, TYPE_SHORT_ASCII,
    TYPE_SHORT_ASCII_INTERNED, TYPE_STRING, TYPE_TRUE, TYPE_UNICODE,
};

#[derive(Clone, Copy, Debug)]
pub(super) enum Scalar<'a> {
    None,
    Bool(bool),
    Ellipsis,
    Int(u64),
    SignedInt(i64),
    BigInt { negative: bool, digits: &'a [u16] },
    Float(f64),
    Complex { real: f64, imag: f64 },
    String { value: &'a str, interned: bool },
    SurrogateString(&'a [u8]),
    Bytes(&'a [u8]),
}

#[expect(
    clippy::inline_always,
    reason = "specializes known scalar variants in the hot marshal path"
)]
#[inline(always)]
pub(super) fn write_scalar(output: &mut Vec<u8>, value: Scalar<'_>, flag: u8) {
    match value {
        Scalar::None => output.push(TYPE_NONE),
        Scalar::Bool(false) => output.push(TYPE_FALSE),
        Scalar::Bool(true) => output.push(TYPE_TRUE),
        Scalar::Ellipsis => output.push(TYPE_ELLIPSIS),
        Scalar::Int(value) => {
            if let Ok(value) = i32::try_from(value) {
                output.push(TYPE_INT | flag);
                output.extend_from_slice(&value.to_le_bytes());
            } else {
                write_long(output, value, false, flag);
            }
        }
        Scalar::SignedInt(value) => {
            if let Ok(value) = i32::try_from(value) {
                output.push(TYPE_INT | flag);
                output.extend_from_slice(&value.to_le_bytes());
            } else {
                write_long(output, value.unsigned_abs(), value.is_negative(), flag);
            }
        }
        Scalar::BigInt { negative, digits } => {
            output.push(TYPE_LONG | flag);
            let digit_count = u32::try_from(digits.len()).expect("marshal integer limit exceeded");
            write_u32(
                output,
                if negative {
                    0_u32.wrapping_sub(digit_count)
                } else {
                    digit_count
                },
            );
            for digit in digits {
                output.extend_from_slice(&digit.to_le_bytes());
            }
        }
        Scalar::Float(value) => {
            output.push(TYPE_BINARY_FLOAT | flag);
            output.extend_from_slice(&value.to_le_bytes());
        }
        Scalar::Complex { real, imag } => {
            output.push(TYPE_BINARY_COMPLEX | flag);
            output.extend_from_slice(&real.to_le_bytes());
            output.extend_from_slice(&imag.to_le_bytes());
        }
        Scalar::String { value, interned } => {
            let bytes = value.as_bytes();
            if value.is_ascii() && bytes.len() < 256 {
                output.push(
                    if interned {
                        TYPE_SHORT_ASCII_INTERNED
                    } else {
                        TYPE_SHORT_ASCII
                    } | flag,
                );
                output
                    .push(u8::try_from(bytes.len()).expect("short ASCII string length fits in u8"));
            } else {
                output.push(
                    if value.is_ascii() {
                        if interned {
                            TYPE_ASCII_INTERNED
                        } else {
                            TYPE_ASCII
                        }
                    } else if interned {
                        TYPE_INTERNED
                    } else {
                        TYPE_UNICODE
                    } | flag,
                );
                write_length(output, bytes.len(), "marshal string exceeds 4 GiB");
            }
            output.extend_from_slice(bytes);
        }
        Scalar::SurrogateString(value) => {
            output.push(TYPE_UNICODE | flag);
            write_length(output, value.len(), "marshal string exceeds 4 GiB");
            output.extend_from_slice(value);
        }
        Scalar::Bytes(value) => {
            output.push(TYPE_STRING | flag);
            write_length(output, value.len(), "marshal value exceeds 4 GiB");
            output.extend_from_slice(value);
        }
    }
}

fn write_long(output: &mut Vec<u8>, magnitude: u64, negative: bool, flag: u8) {
    output.push(TYPE_LONG | flag);
    let digit_count = (64 - magnitude.leading_zeros()).div_ceil(15).max(1);
    write_u32(
        output,
        if negative {
            0_u32.wrapping_sub(digit_count)
        } else {
            digit_count
        },
    );
    let mut remaining = magnitude;
    for _ in 0..digit_count {
        output.extend_from_slice(&((remaining & 0x7fff) as u16).to_le_bytes());
        remaining >>= 15;
    }
}

fn write_length(output: &mut Vec<u8>, length: usize, error: &str) {
    write_u32(output, u32::try_from(length).expect(error));
}

fn write_u32(output: &mut Vec<u8>, value: u32) {
    output.extend_from_slice(&value.to_le_bytes());
}
