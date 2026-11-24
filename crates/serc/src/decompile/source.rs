mod regions;

use ruff_python_ast::str::TripleQuotes;
use ruff_python_literal::escape::{AsciiEscape, UnicodeEscape};

use ruff_python_codegen::{Generator, Indentation};
use ruff_source_file::LineEnding;

use super::instructions::{Bytecode, Instruction};
use crate::PythonVersion;
use crate::marshal::reader::{Code, Id, Module, Object};
use crate::target::operands315::CommonConstant;

// Limit reconstructed text as well as the wire graph. A small shared constant
// can occur at many use sites, and nested code can be shared by reference.
const MAX_SOURCE: usize = 1 << 20;
const MAX_WORK: usize = 16 << 20;

pub(super) fn recover(m: &Module<'_>) -> Result<String, String> {
    let mut work = MAX_WORK;
    let text = scope(m, m.root, 0, &mut work)?;
    let parsed = ruff_python_parser::parse_module(&text)
        .map_err(|e| format!("invalid reconstructed Python: {e}"))?;
    let indent = Indentation::default();
    let mut result = String::new();
    for stmt in parsed.suite() {
        result.push_str(&Generator::new(&indent, LineEnding::Lf).stmt(stmt));
        result.push('\n');
    }
    Ok(result)
}

fn scope(m: &Module<'_>, id: Id, depth: usize, work: &mut usize) -> Result<String, String> {
    if depth > 32 {
        return Err("nested code limit exceeded".into());
    }
    let code = m.code(id)?;
    let name = m.text(code.qualname)?;
    // Location data is optional. Its wire type and first line must still
    // describe a valid code object even when no source ranges were recorded.
    m.bytes(code.locations)?;
    if code.first_line > i32::MAX as usize {
        return Err(format!("{name}: invalid first line"));
    }

    let bc = Bytecode::decode(
        m.version,
        m.bytes(code.bytecode)?,
        m.bytes(code.exceptions)?,
    )
    .map_err(|e| format!("{name}: {e}"))?;
    bc.stack_heights(m.version, code.stack_size)
        .map_err(|e| format!("{name}: {e}"))?;
    if let Some(h) = bc.handlers.first() {
        return Err(format!(
            "{name}: unsupported exception region at byte {}",
            h.start
        ));
    }
    let mut body = Body {
        m,
        code,
        stack: Vec::new(),
        lines: String::new(),
        loop_edges: None,
        depth,
        work,
    };
    body.region(&bc.ops, 0, bc.ops.len(), 0)
        .map_err(|e| format!("{name}: {e}"))?;
    if !body.stack.is_empty() {
        return Err(format!("{name}: unused stack values at end"));
    }
    Ok(body.lines)
}

enum Value {
    Expr(String),
    Null,
    Code(Id),
    Function(Id),
}

struct Body<'m, 'data, 'work> {
    m: &'m Module<'data>,
    code: &'m Code,
    stack: Vec<Value>,
    lines: String,
    loop_edges: Option<(usize, Option<usize>)>,
    depth: usize,
    work: &'work mut usize,
}

impl Body<'_, '_, '_> {
    fn charge(&mut self, cost: usize) -> Result<(), String> {
        *self.work = self
            .work
            .checked_sub(cost)
            .ok_or("reconstruction work limit exceeded")?;
        Ok(())
    }

    fn push(&mut self, expr: String) -> Result<(), String> {
        self.charge(expr.len() + 1)?;
        if expr.len() > MAX_SOURCE {
            return Err("expression size limit exceeded".into());
        }
        // All reconstructed compound expressions carry delimiters until Ruff
        // renders them. Bound nesting before passing text to recursive visitors.
        let mut nesting = 0usize;
        for b in expr.bytes() {
            if matches!(b, b'(' | b'[' | b'{') {
                nesting += 1;
                if nesting > 96 {
                    return Err("expression nesting limit exceeded".into());
                }
            } else if matches!(b, b')' | b']' | b'}') {
                nesting = nesting.saturating_sub(1);
            }
        }
        self.stack.push(Value::Expr(expr));
        Ok(())
    }

    fn line(&mut self, text: &str) -> Result<(), String> {
        self.charge(text.len() + 1)?;
        if self.lines.len() + text.len() >= MAX_SOURCE {
            return Err("source size limit exceeded".into());
        }
        self.lines.push_str(text);
        self.lines.push('\n');
        Ok(())
    }

    fn item(&self, tuple: Id, index: u32) -> Result<Id, String> {
        self.m
            .tuple(tuple)?
            .get(index as usize)
            .copied()
            .ok_or_else(|| "operand index out of range".into())
    }

    fn name(&self, tuple: Id, index: u32) -> Result<&str, String> {
        identifier(self.m.text(self.item(tuple, index)?)?)
    }

    fn pop(&mut self) -> Result<Value, String> {
        self.stack.pop().ok_or_else(|| "stack underflow".into())
    }

    fn expr(&mut self) -> Result<String, String> {
        if let Value::Expr(value) = self.pop()? {
            Ok(value)
        } else {
            Err("expected expression".into())
        }
    }

    fn args(&mut self, n: u32) -> Result<Vec<String>, String> {
        if n as usize > self.stack.len() {
            return Err("too few operands".into());
        }
        let mut args = Vec::new();
        for _ in 0..n {
            args.push(self.expr()?);
        }
        args.reverse();
        Ok(args)
    }

    fn constant(&mut self, arg: u32) -> Result<(), String> {
        let id = self.item(self.code.constants, arg)?;
        if matches!(self.m.objects[id], Object::Code(_)) {
            self.stack.push(Value::Code(id));
            Ok(())
        } else {
            let text = literal(self.m, id, 0, self.work)?;
            self.push(text)
        }
    }

    fn call(&mut self, argc: u32) -> Result<(), String> {
        let args = self.args(argc)?;
        let callable = if self.m.version == PythonVersion::Py312 {
            let name = self.expr()?;
            if !matches!(self.pop()?, Value::Null) {
                return Err("unsupported implicit call argument".into());
            }
            name
        } else {
            if !matches!(self.pop()?, Value::Null) {
                return Err("unsupported implicit call argument".into());
            }
            self.expr()?
        };
        self.push(format!("({callable})({})", args.join(", ")))
    }

    fn load_callable(&mut self, name: String, null: bool) -> Result<(), String> {
        if null && self.m.version == PythonVersion::Py312 {
            self.stack.push(Value::Null);
        }
        self.push(name)?;
        if null && self.m.version != PythonVersion::Py312 {
            self.stack.push(Value::Null);
        }
        Ok(())
    }

    fn store(&mut self, name: &str) -> Result<(), String> {
        match self.pop()? {
            Value::Expr(value) => self.line(&format!("{name} = {value}")),
            Value::Function(id) => {
                let nested = self.m.code(id)?;
                if self.m.text(nested.name)? != name
                    || nested.pos_only > nested.args
                    || nested.kw_only != 0
                {
                    return Err("unsupported function signature".into());
                }
                let locals = self.m.tuple(nested.locals)?;
                let args = locals.get(..nested.args).ok_or("invalid argument count")?;
                let mut names = Vec::new();
                for (i, id) in args.iter().enumerate() {
                    names.push(identifier(self.m.text(*id)?)?);
                    if i + 1 == nested.pos_only {
                        names.push("/");
                    }
                }
                self.line(&format!("def {name}({}):", names.join(", ")))?;
                let text = scope(self.m, id, self.depth + 1, self.work)?;
                if text.is_empty() {
                    self.line("    pass")?;
                }
                for line in text.lines() {
                    self.line(&format!("    {line}"))?;
                }
                Ok(())
            }
            _ => Err("unsupported store operand".into()),
        }
    }

    fn step(&mut self, op: Instruction) -> Result<(), String> {
        self.charge(1)?;
        let a = op.arg;
        match op.name {
            "RESUME" | "NOP" | "NOT_TAKEN" => {}
            "PUSH_NULL" => self.stack.push(Value::Null),
            "LOAD_CONST" => self.constant(a)?,
            "LOAD_SMALL_INT" => self.push(a.to_string())?,
            "LOAD_COMMON_CONSTANT" => {
                let value = [
                    (CommonConstant::NONE, "None"),
                    (CommonConstant::TRUE, "True"),
                    (CommonConstant::FALSE, "False"),
                    (CommonConstant::EMPTY_STR, "''"),
                    (CommonConstant::MINUS_ONE, "-1"),
                ]
                .into_iter()
                .find(|(c, _)| c.argument() == a)
                .ok_or("unsupported common constant")?
                .1;
                self.push(value.to_owned())?;
            }
            "LOAD_NAME" => self.push(self.name(self.code.names, a)?.to_owned())?,
            "LOAD_FAST" | "LOAD_FAST_BORROW" | "LOAD_FAST_CHECK" | "LOAD_DEREF" => {
                self.push(self.name(self.code.locals, a)?.to_owned())?;
            }
            "LOAD_FAST_LOAD_FAST" | "LOAD_FAST_BORROW_LOAD_FAST_BORROW" => {
                for index in [a >> 4, a & 15] {
                    self.push(self.name(self.code.locals, index)?.to_owned())?;
                }
            }
            "LOAD_GLOBAL" => {
                self.load_callable(self.name(self.code.names, a >> 1)?.to_owned(), a & 1 != 0)?;
            }
            "LOAD_ATTR" => {
                let obj = self.expr()?;
                let attr = self.name(self.code.names, a >> 1)?;
                self.load_callable(format!("({obj}).{attr}"), a & 1 != 0)?;
            }
            "STORE_NAME" | "STORE_FAST" | "STORE_DEREF" => {
                let tuple = if op.name == "STORE_NAME" {
                    self.code.names
                } else {
                    self.code.locals
                };
                let name = self.name(tuple, a)?.to_owned();
                self.store(&name)?;
            }
            "POP_TOP" => {
                let expr = self.expr()?;
                self.line(&expr)?;
            }
            "CALL" => self.call(a)?,
            "MAKE_FUNCTION" if a == 0 => {
                let Value::Code(id) = self.pop()? else {
                    return Err("expected nested code".into());
                };
                self.stack.push(Value::Function(id));
            }
            "BINARY_OP" | "BINARY_SUBSCR" | "COMPARE_OP" | "IS_OP" | "CONTAINS_OP" => {
                let rhs = self.expr()?;
                let lhs = self.expr()?;
                let operator = match op.name {
                    "IS_OP" => {
                        if a == 0 {
                            "is"
                        } else {
                            "is not"
                        }
                    }
                    "CONTAINS_OP" => {
                        if a == 0 {
                            "in"
                        } else {
                            "not in"
                        }
                    }
                    "COMPARE_OP" => {
                        let shift = if self.m.version == PythonVersion::Py312 {
                            4
                        } else {
                            5
                        };
                        ["<", "<=", "==", "!=", ">", ">="]
                            .get((a >> shift) as usize)
                            .copied()
                            .ok_or("unknown comparison")?
                    }
                    "BINARY_SUBSCR" => "[]",
                    _ => *[
                        "+", "&", "//", "<<", "@", "*", "%", "|", "**", ">>", "-", "/", "^",
                    ]
                    .get(a as usize)
                    .or_else(|| (a == 26).then_some(&"[]"))
                    .ok_or("unsupported binary operator")?,
                };
                if operator == "[]" {
                    self.push(format!("({lhs})[{rhs}]"))?;
                } else {
                    self.push(format!("({lhs} {operator} {rhs})"))?;
                }
            }
            "UNARY_NEGATIVE" | "UNARY_INVERT" | "UNARY_NOT" => {
                let value = self.expr()?;
                let sign = match op.name {
                    "UNARY_NEGATIVE" => "-",
                    "UNARY_INVERT" => "~",
                    _ => "not ",
                };
                self.push(format!("({sign}{value})"))?;
            }
            "BUILD_LIST" | "BUILD_TUPLE" | "BUILD_SET" => {
                let args = self.args(a)?.join(", ");
                let expr = match op.name {
                    "BUILD_LIST" => format!("[{args}]"),
                    "BUILD_SET" if a > 0 => format!("{{{args}}}"),
                    "BUILD_TUPLE" => {
                        if a == 1 {
                            format!("({args},)")
                        } else {
                            format!("({args})")
                        }
                    }
                    _ => return Err("unsupported empty set".into()),
                };
                self.push(expr)?;
            }
            "RETURN_CONST" | "RETURN_VALUE" => {
                if op.name == "RETURN_CONST" {
                    self.constant(a)?;
                }
                let expr = self.expr()?;
                if self.depth == 0 {
                    if expr != "None" {
                        return Err("module returns a value".into());
                    }
                } else {
                    self.line(&format!("return {expr}"))?;
                }
            }
            _ => return Err(format!("unsupported {}", op.name)),
        }
        Ok(())
    }
}

fn identifier(name: &str) -> Result<&str, String> {
    if name.len() > 1024
        || name.is_empty()
        || !name.chars().all(|c| c == '_' || c.is_alphanumeric())
    {
        return Err("unsupported identifier".into());
    }
    Ok(name)
}

fn literal(m: &Module<'_>, id: Id, depth: usize, work: &mut usize) -> Result<String, String> {
    if depth > 32 || *work == 0 {
        return Err("constant expansion limit exceeded".into());
    }
    *work -= 1;
    let result = match &m.objects[id] {
        Object::None => "None".into(),
        Object::Bool(v) => if *v { "True" } else { "False" }.into(),
        Object::Ellipsis => "...".into(),
        Object::Int(i) => i.to_string(),
        Object::Float(bits) if f64::from_bits(*bits).is_finite() => {
            format!("{:?}", f64::from_bits(*bits))
        }
        Object::Bytes(bytes) => {
            if bytes.len() > MAX_SOURCE / 4 {
                return Err("bytes literal limit exceeded".into());
            }
            AsciiEscape::new_repr(bytes)
                .bytes_repr(TripleQuotes::No)
                .to_string()
                .ok_or("bytes representation overflow")?
        }
        Object::Text { .. } => {
            let text = m.text(id)?;
            if text.len() > MAX_SOURCE / 6 {
                return Err("string literal limit exceeded".into());
            }
            UnicodeEscape::new_repr(text)
                .str_repr(TripleQuotes::No)
                .to_string()
                .ok_or("string representation overflow")?
        }
        Object::Tuple(items) => {
            let mut s = String::from("(");
            for item in items {
                s.push_str(&literal(m, *item, depth + 1, work)?);
                s.push_str(", ");
                if s.len() > MAX_SOURCE {
                    return Err("tuple literal limit exceeded".into());
                }
            }
            s.push(')');
            s
        }
        _ => return Err("unsupported literal".into()),
    };
    *work = work
        .checked_sub(result.len())
        .ok_or("constant expansion limit exceeded")?;
    Ok(result)
}
