//! Source operands shared across old and new instruction layouts.
use super::{
    Body, Instruction, Value,
    definitions::{string_value, tuple_items},
    identifier,
};

fn slice_text(items: &[String]) -> String {
    items
        .iter()
        .map(|s| if s == "None" { "" } else { s })
        .collect::<Vec<_>>()
        .join(":")
}

impl Body<'_, '_, '_> {
    pub(super) fn merge_map(&mut self) -> Result<(), String> {
        let other = self.expr()?;
        let map = self.expr()?;
        let prefix = map
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .ok_or("unsupported map update")?;
        let separator = if prefix.is_empty() { "" } else { ", " };
        self.push(format!("{{{prefix}{separator}**({other})}}"))
    }

    pub(super) fn expanded_call(&mut self, flag: u32) -> Result<(), String> {
        let keywords = if self.m.version >= crate::PythonVersion::Py314 {
            if matches!(self.stack.last(), Some(Value::Null)) {
                self.pop()?;
                None
            } else {
                Some(self.expr()?)
            }
        } else if flag & 1 != 0 {
            Some(self.expr()?)
        } else {
            None
        };
        let positional = self.expr()?;
        let mut args =
            tuple_items(&positional).unwrap_or_else(|_| vec![format!("*({positional})")]);
        if let Some(keywords) = keywords {
            let parsed =
                ruff_python_parser::parse_expression(&keywords).map_err(|e| e.to_string())?;
            if let ruff_python_ast::Expr::Dict(map) = parsed.expr() {
                for item in &map.items {
                    let value = super::definitions::render(&item.value);
                    match &item.key {
                        Some(ruff_python_ast::Expr::StringLiteral(key)) => {
                            args.push(format!("{}={value}", identifier(key.value.to_str())?));
                        }
                        None => args.push(format!("**({value})")),
                        _ => return Err("unsupported expanded keyword".into()),
                    }
                }
            } else {
                args.push(format!("**({keywords})"));
            }
        }
        self.emit_call(&args)
    }

    pub(super) fn keyword_call(&mut self, argc: u32, keys: &str) -> Result<(), String> {
        let keys = tuple_items(keys)?;
        let mut args = self.args(argc)?;
        let first = args
            .len()
            .checked_sub(keys.len())
            .ok_or("invalid keyword argument count")?;
        for (arg, key) in args[first..].iter_mut().zip(keys) {
            let key = string_value(&key)?;
            *arg = format!("{}={arg}", identifier(&key)?);
        }
        self.emit_call(&args)
    }

    pub(super) fn extend_literal(&mut self, op: Instruction) -> Result<(), String> {
        if op.arg != 1 {
            return Err("unsupported collection update".into());
        }
        let items = self.expr()?;
        let collection = self.expr()?;
        let (open, close) = if op.name == "LIST_EXTEND" {
            ('[', ']')
        } else {
            ('{', '}')
        };
        let prefix = collection
            .strip_prefix(open)
            .and_then(|s| s.strip_suffix(close))
            .ok_or("unsupported collection update")?;
        let prefix = if prefix.is_empty() || prefix == "*()" {
            String::new()
        } else {
            format!("{prefix}, ")
        };
        let body = tuple_items(&items).map_or_else(|_| format!("*({items})"), |v| v.join(", "));
        self.push(format!("{open}{prefix}{body}{close}"))
    }

    pub(super) fn unpack_names(
        &mut self,
        ops: &[Instruction],
        i: usize,
        end: usize,
    ) -> Result<usize, String> {
        let value = self.expr()?;
        let mut next = i + 1;
        let mut names = Vec::new();
        let count = ops[i].arg as usize;
        while names.len() < count && next < end {
            let store = ops[next];
            match store.name {
                "STORE_FAST" | "STORE_NAME" | "STORE_DEREF" => {
                    let table = if store.name == "STORE_NAME" {
                        self.code.names
                    } else {
                        self.code.locals
                    };
                    names.push(self.name(table, store.arg)?.to_owned());
                }
                "STORE_FAST_STORE_FAST" => {
                    for index in [store.arg >> 4, store.arg & 15] {
                        names.push(self.name(self.code.locals, index)?.to_owned());
                    }
                }
                _ => return Err("unsupported unpacking target".into()),
            }
            next += 1;
        }
        if count == 0 || names.len() != count {
            return Err("invalid unpacked target count".into());
        }
        self.line(&format!("({},) = {value}", names.join(", ")))?;
        Ok(next)
    }

    pub(super) fn index(&mut self) -> Result<String, String> {
        if matches!(self.stack.last(), Some(Value::Slice(_))) {
            let Value::Slice(text) = self.pop()? else {
                unreachable!()
            };
            Ok(text)
        } else {
            self.expr()
        }
    }

    pub(super) fn subscript_or_attribute(&mut self, op: Instruction) -> Result<(), String> {
        if op.name == "STORE_ATTR" {
            let obj = self.expr()?;
            let value = self.expr()?;
            let name = self.name(self.code.names, op.arg)?;
            return self.line(&format!("({obj}).{name} = {value}"));
        }
        if op.name == "BUILD_SLICE" {
            if !matches!(op.arg, 2 | 3) {
                return Err("invalid slice arity".into());
            }
            let items = self.args(op.arg)?;
            self.stack.push(Value::Slice(slice_text(&items)));
            return Ok(());
        }
        let index = if op.name == "STORE_SUBSCR" {
            self.index()?
        } else {
            slice_text(&self.args(2)?)
        };
        let obj = self.expr()?;
        if op.name == "BINARY_SLICE" {
            return self.push(format!("({obj})[{index}]"));
        }
        let value = self.expr()?;
        if obj == "__annotations__" {
            let binding = string_value(&index)?;
            let name = identifier(&binding)?;
            let annotation =
                if self.code.flags & crate::target::code_flags::CO_FUTURE_ANNOTATIONS != 0 {
                    string_value(&value)?
                } else {
                    value
                };
            return self.annotate(name, &annotation);
        }
        self.line(&format!("({obj})[{index}] = {value}"))
    }
}

pub(super) fn constant_slice(
    body: &mut Body<'_, '_, '_>,
    ids: [super::Id; 3],
) -> Result<(), String> {
    let mut items = Vec::new();
    for id in ids {
        items.push(super::literal(body.m, id, 0, &mut body.work.budget)?);
    }
    if items[2] == "None" {
        items.pop();
    }
    body.stack.push(Value::Slice(slice_text(&items)));
    Ok(())
}
