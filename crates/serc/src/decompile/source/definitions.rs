use ruff_python_ast::{Expr, str::TripleQuotes};
use ruff_python_codegen::{Generator, Indentation};
use ruff_python_literal::escape::UnicodeEscape;
use ruff_source_file::LineEnding;

use super::{Body, Code, Id, Instruction, Object, Value, identifier, literal, scope};
use crate::PythonVersion;
use crate::target::{code_flags, local_kinds};

pub(super) struct Definition {
    pub(super) id: Id,
    defaults: Vec<String>,
    keywords: Vec<(String, String)>,
    decorators: Vec<String>,
    annotations: Vec<(String, String)>,
    type_params: Vec<String>,
}

pub(super) fn string_value(text: &str) -> Result<String, String> {
    let parsed = ruff_python_parser::parse_expression(text).map_err(|err| err.to_string())?;
    let Expr::StringLiteral(value) = parsed.expr() else {
        return Err("expected string literal".into());
    };
    Ok(value.value.to_str().to_owned())
}

pub(super) fn tuple_items(text: &str) -> Result<Vec<String>, String> {
    let expression = ruff_python_parser::parse_expression(text).map_err(|err| err.to_string())?;
    let Expr::Tuple(tuple) = expression.expr() else {
        return Err("expected tuple operand".into());
    };
    Ok(tuple.elts.iter().map(render).collect())
}

pub(super) fn render(expr: &Expr) -> String {
    Generator::new(&Indentation::default(), LineEnding::Lf).expr(expr)
}

pub(super) struct Import {
    path: String,
    pub(super) members: Vec<(String, String)>,
    from: bool,
    lazy: bool,
}

impl Import {
    pub(super) fn statement(&self, binding: &str) -> Result<String, String> {
        let prefix = if self.lazy { "lazy " } else { "" };
        if self.from {
            if self.members.is_empty() {
                return Err("unsupported from-import".into());
            }
            let members: Vec<_> = self
                .members
                .iter()
                .map(|(name, alias)| {
                    if name == alias {
                        name.clone()
                    } else {
                        format!("{name} as {alias}")
                    }
                })
                .collect();
            Ok(format!(
                "{prefix}from {} import {}",
                self.path,
                members.join(", ")
            ))
        } else if let Some((member, alias)) = self.members.last() {
            if self.path.split('.').next_back() != Some(member) {
                return Err("unsupported dotted import binding".into());
            }
            Ok(format!("{prefix}import {} as {alias}", self.path))
        } else if self.path.split('.').next() == Some(binding) {
            Ok(format!("{prefix}import {}", self.path))
        } else {
            Ok(format!("{prefix}import {} as {binding}", self.path))
        }
    }
}

impl Body<'_, '_, '_> {
    pub(super) fn import(&mut self, arg: u32) -> Result<(), String> {
        let from = self.expr()? != "None";
        let level: usize = self.expr()?.parse().map_err(|_| "invalid import level")?;
        if level > 255 {
            return Err("relative import too deep".into());
        }
        let (index, lazy) = if self.m.version == PythonVersion::Py315 {
            (
                arg >> 2,
                arg & 3 == crate::target::operands315::ImportMode::LAZY.argument(),
            )
        } else {
            (arg, false)
        };
        let path = self.m.text(self.item(self.code.names, index)?)?;
        if !path.is_empty() {
            for part in path.split('.') {
                identifier(part)?;
            }
        }
        self.stack.push(Value::Import(Import {
            path: format!("{}{path}", ".".repeat(level)),
            from,
            lazy,
            members: Vec::new(),
        }));
        Ok(())
    }

    pub(super) fn declarations(&mut self, ops: &[Instruction]) -> Result<(), String> {
        if self.depth == 0 {
            return Ok(());
        }
        if !self.class
            && let Some(&id) = self.m.tuple(self.code.constants)?.first()
            && matches!(self.m.objects[id], Object::Text { .. })
            && (self.m.version <= PythonVersion::Py313
                || self.code.flags & code_flags::CO_HAS_DOCSTRING != 0)
        {
            let doc = literal(self.m, id, 0, &mut self.work.budget)?;
            self.line(&doc)?;
        }
        let kinds = self.m.bytes(self.code.local_kinds)?;
        let mut declarations = std::collections::BTreeSet::new();
        for op in ops {
            if op.name == "STORE_GLOBAL" {
                declarations.insert(format!("global {}", self.name(self.code.names, op.arg)?));
            }
            if op.name == "STORE_DEREF"
                && kinds
                    .get(op.arg as usize)
                    .is_some_and(|kind| kind & local_kinds::CO_FAST_FREE != 0)
            {
                declarations.insert(format!("nonlocal {}", self.name(self.code.locals, op.arg)?));
            }
        }
        for decl in declarations {
            self.line(&decl)?;
        }
        Ok(())
    }

    pub(super) fn build_map(&mut self, op: &str, count: u32) -> Result<(), String> {
        let pairs = if op == "BUILD_CONST_KEY_MAP" {
            let keys = tuple_items(&self.expr()?)?;
            if keys.len() != count as usize {
                return Err("invalid constant map keys".into());
            }
            keys.into_iter().zip(self.args(count)?).collect::<Vec<_>>()
        } else {
            let mut pairs = Vec::new();
            for _ in 0..count {
                let v = self.expr()?;
                pairs.push((self.expr()?, v));
            }
            pairs.reverse();
            pairs
        };
        let parts: Vec<_> = pairs
            .into_iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect();
        self.push(format!("{{{}}}", parts.join(", ")))
    }

    pub(super) fn make_function(&mut self, flags: u32) -> Result<(), String> {
        let Value::Code(id) = self.pop()? else {
            return Err("expected function code".into());
        };
        let mut def = Definition {
            id,
            defaults: Vec::new(),
            keywords: Vec::new(),
            decorators: Vec::new(),
            annotations: Vec::new(),
            type_params: Vec::new(),
        };
        if flags & !15 != 0 {
            return Err("unknown function flags".into());
        }
        for flag in [8, 4, 2, 1] {
            if flags & flag != 0 {
                self.attribute(&mut def, flag)?;
            }
        }
        self.stack.push(Value::Function(def));
        Ok(())
    }

    pub(super) fn set_attribute(&mut self, flag: u32) -> Result<(), String> {
        let Value::Function(mut def) = self.pop()? else {
            return Err("expected function".into());
        };
        self.attribute(&mut def, flag)?;
        self.stack.push(Value::Function(def));
        Ok(())
    }

    fn attribute(&mut self, def: &mut Definition, flag: u32) -> Result<(), String> {
        if flag == 16 {
            let Value::Function(annotation) = self.pop()? else {
                return Err("expected annotation function".into());
            };
            def.annotations = self.lazy_annotations(annotation.id)?;
            self.annotation_spellings(def)?;
            return Ok(());
        }
        let text = self.expr()?;
        match flag {
            4 => {
                let items = tuple_items(&text)?;
                if !items.len().is_multiple_of(2) {
                    return Err("unpaired annotations".into());
                }
                for pair in items.chunks_exact(2) {
                    let parsed = ruff_python_parser::parse_expression(&pair[0])
                        .map_err(|e| e.to_string())?;
                    let Expr::StringLiteral(key) = parsed.expr() else {
                        return Err("expected annotation name".into());
                    };
                    def.annotations
                        .push((key.value.to_str().to_owned(), pair[1].clone()));
                }
                self.annotation_spellings(def)?;
            }
            8 => {
                tuple_items(&text)?;
            }
            1 => def.defaults = tuple_items(&text)?,
            2 => {
                let expression =
                    ruff_python_parser::parse_expression(&text).map_err(|e| e.to_string())?;
                let Expr::Dict(dict) = expression.expr() else {
                    return Err("invalid keyword defaults".into());
                };
                for item in &dict.items {
                    let Some(Expr::StringLiteral(key)) = &item.key else {
                        return Err("invalid parameter name".into());
                    };
                    def.keywords.push((
                        identifier(key.value.to_str())?.to_owned(),
                        render(&item.value),
                    ));
                }
            }
            _ => return Err(format!("unsupported function attribute {flag}")),
        }
        Ok(())
    }

    fn annotation_spellings(&self, def: &mut Definition) -> Result<(), String> {
        if self.code.flags & code_flags::CO_FUTURE_ANNOTATIONS != 0 {
            for (_, annotation) in &mut def.annotations {
                *annotation = string_value(annotation)?;
            }
        }
        Ok(())
    }

    pub(super) fn special_call(&mut self, argc: u32) -> Result<Option<Value>, String> {
        if argc == 0 && self.stack.len() >= 2 {
            let slot = self.stack.len() - 2;
            if let [Value::Function(def), Value::Expr(_)] = &self.stack[slot..]
                && self.m.text(self.m.code(def.id)?.name)? == "<genexpr>"
            {
                let id = def.id;
                let iterable = self.expr()?;
                self.pop()?;
                let (mut body, bc) = self.wrapper(id)?;
                body.stack.push(Value::Expr(iterable));
                body.comprehension(&bc.ops, 0, bc.ops.len(), 0)?;
                return Ok(Some(body.pop()?));
            }
            let generic = match &self.stack[slot..] {
                [Value::Function(def), Value::Null] | [Value::Null, Value::Function(def)] => {
                    Some(def.id)
                }
                _ => None,
            };
            if let Some(id) = generic
                && self
                    .m
                    .text(self.m.code(id)?.name)?
                    .starts_with("<generic parameters of ")
            {
                let def = self.generic(id)?;
                self.stack.truncate(slot);
                return Ok(Some(Value::Function(def)));
            }
        }
        if argc == 0 && matches!(self.stack.last(), Some(Value::Function(_))) {
            let Value::Function(mut def) = self.pop()? else {
                unreachable!()
            };
            def.decorators.push(self.expr()?);
            return Ok(Some(Value::Function(def)));
        }
        let Some(start) = self.stack.len().checked_sub(argc as usize + 2) else {
            return Ok(None);
        };
        let is_class = if self.m.version == PythonVersion::Py312 {
            matches!(
                &self.stack[start..start + 2],
                [Value::Null, Value::BuildClass]
            )
        } else {
            matches!(
                &self.stack[start..start + 2],
                [Value::BuildClass, Value::Null]
            )
        };
        if !is_class {
            return Ok(None);
        }
        let bases = self.args(argc.checked_sub(2).ok_or("missing class arguments")?)?;
        let name = self.expr()?;
        let Value::Function(def) = self.pop()? else {
            return Err("expected class body".into());
        };
        if UnicodeEscape::new_repr(self.m.text(self.m.code(def.id)?.name)?)
            .str_repr(TripleQuotes::No)
            .to_string()
            .as_deref()
            != Some(&name)
        {
            return Err("unsupported dynamic class name".into());
        }
        self.stack.truncate(start);
        Ok(Some(Value::Class(def.id, bases)))
    }

    pub(super) fn class_definition(
        &mut self,
        name: &str,
        id: Id,
        bases: &[String],
    ) -> Result<(), String> {
        let body = scope(self.m, id, self.depth + 1, true, self.work)?;
        self.block(&format!("class {name}({}):", bases.join(", ")), &body)
    }

    pub(super) fn definition(&mut self, name: &str, def: &Definition) -> Result<(), String> {
        let code = self.m.code(def.id)?;
        if self.m.text(code.name)? == "<lambda>" {
            let expr = self.lambda(def)?;
            return self.line(&format!("{name} = {expr}"));
        }
        if self.m.text(code.name)? != name {
            return Err("function binding differs from code name".into());
        }
        let params = self.parameters(code, def)?;
        let body = scope(self.m, def.id, self.depth + 1, false, self.work)?;
        for decorator in def.decorators.iter().rev() {
            self.line(&format!("@{decorator}"))?;
        }
        let generics = if def.type_params.is_empty() {
            String::new()
        } else {
            format!("[{}]", def.type_params.join(", "))
        };
        let result = def
            .annotations
            .iter()
            .find(|(k, _)| k == "return")
            .map_or_else(String::new, |(_, v)| format!(" -> ({v})"));
        let prefix =
            if code.flags & (code_flags::CO_COROUTINE | code_flags::CO_ASYNC_GENERATOR) == 0 {
                ""
            } else {
                "async "
            };
        self.block(
            &format!("{prefix}def {name}{generics}({params}){result}:"),
            &body,
        )
    }

    pub(super) fn lambda(&mut self, def: &Definition) -> Result<String, String> {
        let code = self.m.code(def.id)?;
        if self.m.text(code.name)? != "<lambda>" {
            return Err("expected expression".into());
        }
        let params = self.parameters(code, def)?;
        let body = scope(self.m, def.id, self.depth + 1, false, self.work)?;
        let expr = body
            .trim_end()
            .strip_prefix("return ")
            .filter(|s| !s.contains('\n'))
            .ok_or("unsupported lambda body")?;
        Ok(format!("(lambda {params}: {expr})"))
    }

    pub(super) fn import_star(&mut self) -> Result<(), String> {
        let Value::Import(mut import) = self.pop()? else {
            return Err("expected star import".into());
        };
        import.members.push(("*".into(), "*".into()));
        self.line(&import.statement("")?)
    }

    fn parameters(&self, code: &Code, def: &Definition) -> Result<String, String> {
        let locals = self.m.tuple(code.locals)?;
        let param = |i| -> Result<&str, String> {
            identifier(
                self.m
                    .text(*locals.get(i).ok_or("invalid parameter index")?)?,
            )
        };
        let start_defaults = code
            .args
            .checked_sub(def.defaults.len())
            .ok_or("too many defaults")?;
        if code.pos_only > code.args || code.args + code.kw_only > locals.len() {
            return Err("invalid signature".into());
        }
        let annotate = |name: &str| {
            def.annotations
                .iter()
                .find(|(key, _)| key == name)
                .map_or_else(|| name.to_owned(), |(_, v)| format!("{name}: {v}"))
        };
        let mut parts = Vec::new();
        for i in 0..code.args {
            let name = annotate(param(i)?);
            parts.push(if i < start_defaults {
                name
            } else {
                format!("{name}={}", def.defaults[i - start_defaults])
            });
            if i + 1 == code.pos_only {
                parts.push("/".into());
            }
        }
        let mut extra = code.args + code.kw_only;
        if code.flags & code_flags::CO_VARARGS != 0 {
            parts.push(format!("*{}", annotate(param(extra)?)));
            extra += 1;
        } else if code.kw_only != 0 {
            parts.push("*".into());
        }
        for i in code.args..code.args + code.kw_only {
            let name = param(i)?;
            parts.push(
                if let Some((_, default)) = def.keywords.iter().find(|(k, _)| k == name) {
                    format!("{}={default}", annotate(name))
                } else {
                    annotate(name)
                },
            );
        }
        if code.flags & code_flags::CO_VARKEYWORDS != 0 {
            parts.push(format!("**{}", annotate(param(extra)?)));
        }
        Ok(parts.join(", "))
    }
}

impl Body<'_, '_, '_> {
    pub(super) fn wrapper(
        &mut self,
        id: Id,
    ) -> Result<(Body<'_, '_, '_>, super::Bytecode), String> {
        if self.depth >= 32 {
            return Err("wrapper nesting limit exceeded".into());
        }
        let code = self.m.code(id)?;
        let bc = super::Bytecode::decode(
            self.m.version,
            self.m.bytes(code.bytecode)?,
            self.m.bytes(code.exceptions)?,
        )?;
        bc.stack_heights(self.m.version, code.stack_size)?;
        if !bc.handlers.is_empty() && self.m.text(code.name)? != "<genexpr>" {
            return Err("unsupported wrapper exception region".into());
        }
        Ok((
            Body {
                m: self.m,
                code,
                stack: Vec::new(),
                lines: String::new(),
                loop_edges: None,
                class: false,
                annotations: Vec::new(),
                depth: self.depth + 1,
                work: self.work,
            },
            bc,
        ))
    }

    fn lazy_annotations(&mut self, id: Id) -> Result<Vec<(String, String)>, String> {
        let (mut body, bc) = self.wrapper(id)?;
        let branch = bc
            .ops
            .iter()
            .find(|op| op.name == "POP_JUMP_IF_FALSE")
            .and_then(|op| op.jump)
            .ok_or("unsupported annotation guard")?;
        let start = bc
            .ops
            .binary_search_by_key(&branch, |op| op.offset)
            .map_err(|_| "invalid annotation target")?;
        let end = bc.ops.len() - 1;
        if bc.ops[end].name != "RETURN_VALUE" {
            return Err("unsupported annotation result".into());
        }
        body.region(&bc.ops, start, end, 0)?;
        let result = body.expr()?;
        let parsed = ruff_python_parser::parse_expression(&result).map_err(|e| e.to_string())?;
        let Expr::Dict(map) = parsed.expr() else {
            return Err("expected annotation map".into());
        };
        map.items
            .iter()
            .map(|item| {
                let Some(Expr::StringLiteral(key)) = &item.key else {
                    return Err("expected annotation key".into());
                };
                Ok((key.value.to_str().to_owned(), render(&item.value)))
            })
            .collect()
    }

    fn generic(&mut self, id: Id) -> Result<Definition, String> {
        use crate::target::operands::UnaryIntrinsic;
        let (mut body, bc) = self.wrapper(id)?;
        let tuple = bc
            .ops
            .iter()
            .position(|op| op.name == "BUILD_TUPLE")
            .ok_or("unsupported type parameter tuple")?;
        let mut params = Vec::new();
        for pair in bc.ops[..tuple].windows(2) {
            if pair[0].name == "LOAD_CONST" && pair[1].name == "CALL_INTRINSIC_1" {
                let name = identifier(body.m.text(body.item(body.code.constants, pair[0].arg)?)?)?;
                let prefix = if pair[1].arg == UnaryIntrinsic::TYPEVAR.argument() {
                    ""
                } else if pair[1].arg == UnaryIntrinsic::PARAMSPEC.argument() {
                    "**"
                } else if pair[1].arg == UnaryIntrinsic::TYPEVARTUPLE.argument() {
                    "*"
                } else {
                    return Err("unsupported type parameter bound".into());
                };
                params.push(format!("{prefix}{name}"));
            }
        }
        if params.len() != bc.ops[tuple].arg as usize {
            return Err("unsupported generic parameter layout".into());
        }
        let swap = bc
            .ops
            .iter()
            .rposition(|op| op.name == "SWAP")
            .ok_or("unsupported generic result")?;
        body.region(&bc.ops, tuple + 1, swap, 0)?;
        let Value::Function(mut def) = body.pop()? else {
            return Err("expected generic function".into());
        };
        def.type_params = params;
        Ok(def)
    }
}
