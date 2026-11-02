//! Keep interpolation boundaries until `BUILD_STRING` / `BUILD_TEMPLATE` so literal
//! braces, conversions and format specs survive recompilation.
use std::fmt::Write as _;

use super::{
    Body, Instruction, Value,
    definitions::{string_value, tuple_items},
};

fn conversion(arg: u32) -> Result<&'static str, String> {
    ["", "!s", "!r", "!a"]
        .get(arg as usize)
        .copied()
        .ok_or_else(|| "unknown string conversion".into())
}

fn escaped(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            ch if ch.is_control() => write!(out, "\\x{:02x}", u32::from(ch)).unwrap(),
            ch => out.push(ch),
        }
    }
    out
}

impl Body<'_, '_, '_> {
    pub(super) fn interpolations(&mut self, count: u32) -> Result<Vec<String>, String> {
        let mut fields = Vec::new();
        for _ in 0..count {
            let Value::Interpolation(field) = self.pop()? else {
                return Err("invalid template interpolation tuple".into());
            };
            fields.push(field);
        }
        fields.reverse();
        Ok(fields)
    }

    fn string_part(&mut self) -> Result<String, String> {
        match self.pop()? {
            Value::Formatted(text) => Ok(text),
            Value::Expr(literal) => Ok(escaped(&string_value(&literal)?)),
            _ => Err("expected literal string or interpolation".into()),
        }
    }

    pub(super) fn string_op(&mut self, op: Instruction) -> Result<(), String> {
        let a = op.arg;
        match op.name {
            "CONVERT_VALUE" => {
                let value = self.expr()?;
                self.stack.push(Value::Converted(value, conversion(a)?));
            }
            "FORMAT_VALUE" | "FORMAT_WITH_SPEC" | "FORMAT_SIMPLE" | "BUILD_INTERPOLATION" => {
                let template = op.name == "BUILD_INTERPOLATION";
                let has_spec = op.name == "FORMAT_WITH_SPEC"
                    || (op.name == "FORMAT_VALUE" && a & 4 != 0)
                    || (template && a & 1 != 0);
                let spec = if has_spec {
                    format!(":{}", self.string_part()?)
                } else {
                    String::new()
                };
                let (value, conv) = if template {
                    let spelling = string_value(&self.expr()?)?;
                    self.expr()?;
                    (spelling, conversion(a >> 2)?)
                } else if matches!(self.stack.last(), Some(Value::Converted(..))) {
                    let Value::Converted(value, conv) = self.pop()? else {
                        unreachable!()
                    };
                    (value, conv)
                } else {
                    (
                        self.expr()?,
                        conversion(if op.name == "FORMAT_VALUE" { a & 3 } else { 0 })?,
                    )
                };
                let field = format!("{{{value}{conv}{spec}}}");
                self.charge(field.len())?;
                self.stack.push(if template {
                    Value::Interpolation(field)
                } else {
                    Value::Formatted(field)
                });
            }
            "BUILD_STRING" => {
                let mut parts = Vec::new();
                for _ in 0..a {
                    parts.push(self.string_part()?);
                }
                parts.reverse();
                let text = parts.concat();
                self.charge(text.len())?;
                self.stack.push(Value::Formatted(text));
            }
            "BUILD_TEMPLATE" => {
                let fields = match self.pop()? {
                    Value::Interpolations(fields) => fields,
                    Value::Expr(tuple) if tuple_items(&tuple)?.is_empty() => Vec::new(),
                    _ => return Err("missing template interpolations".into()),
                };
                let strings = tuple_items(&self.expr()?)?;
                if strings.len() != fields.len() + 1 {
                    return Err("invalid template segment count".into());
                }
                let mut text = String::from("t\"");
                for (index, s) in strings.iter().enumerate() {
                    text.push_str(&escaped(&string_value(s)?));
                    if let Some(field) = fields.get(index) {
                        text.push_str(field);
                    }
                }
                text.push('"');
                self.push(text)?;
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
