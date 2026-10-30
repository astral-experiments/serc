mod annotations;
mod definitions;
mod exceptions;
mod operands;
mod patterns;
mod regions;
mod strings;
mod suspension;
use definitions::{Definition, Import};

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

struct Recovery {
    budget: usize,
    preserve_nops: bool,
    saw_nop: bool,
}

pub(super) fn recover(m: &Module<'_>, preserve_nops: bool) -> Result<(Vec<String>, bool), String> {
    let mut work = Recovery {
        budget: MAX_WORK,
        preserve_nops,
        saw_nop: false,
    };
    let text = scope(m, m.root, 0, false, &mut work)?;
    let parsed = ruff_python_parser::parse_module(&text)
        .map_err(|e| format!("invalid reconstructed Python: {e}"))?;
    let indent = Indentation::default();
    let mut result = String::new();
    for stmt in parsed.suite() {
        result.push_str(&Generator::new(&indent, LineEnding::Lf).stmt(stmt));
        result.push('\n');
    }
    let mut candidates = vec![result];
    if candidates[0] != text {
        // Suite layout can change tracing NOPs even when the AST is identical.
        candidates.push(text);
    }
    if m.version >= PythonVersion::Py313 {
        // Class bodies store __firstlineno__ as an executable constant.
        // Restore blank lines before unambiguous class definitions when needed.
        let mut class_lines = rustc_hash::FxHashMap::default();
        for obj in &m.objects {
            if let Object::Code(code) = obj
                && code.flags & crate::target::code_flags::CO_NEWLOCALS == 0
            {
                class_lines
                    .entry(m.text(code.name)?)
                    .and_modify(|line| *line = None)
                    .or_insert(Some(code.first_line));
            }
        }
        for i in 0..candidates.len() {
            if let Some(layout) = class_layout(&candidates[i], &class_lines) {
                candidates.push(layout);
            }
        }
    }
    Ok((candidates, work.saw_nop))
}

fn class_layout(
    source: &str,
    lines: &rustc_hash::FxHashMap<&str, Option<usize>>,
) -> Option<String> {
    let mut result = String::new();
    let mut lineno = 1;
    for line in source.split_inclusive('\n') {
        if let Some(name) = line
            .trim_start()
            .strip_prefix("class ")
            .and_then(|s| s.split(['(', '[', ':']).next())
            && let Some(&Some(target)) = lines.get(name)
            && target > lineno
        {
            let gap = target - lineno;
            if gap.checked_add(result.len())?.checked_add(source.len())? > MAX_SOURCE {
                return None;
            }
            result.extend(std::iter::repeat_n('\n', gap));
            lineno = target;
        }
        result.push_str(line);
        lineno += 1;
    }
    (result.len() != source.len()).then_some(result)
}

fn scope(
    m: &Module<'_>,
    id: Id,
    depth: usize,
    class: bool,
    work: &mut Recovery,
) -> Result<String, String> {
    if crate::stack_growth_required() {
        return crate::grow_stack(|| scope(m, id, depth, class, work));
    }
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
    crate::decompile::flow::stack_heights(&bc, m.version, code.stack_size)
        .map_err(|e| format!("{name}: {e}"))?;
    let mut body = Body {
        m,
        code,
        stack: Vec::new(),
        lines: String::new(),
        loop_edges: None,
        class,
        annotations: Vec::new(),
        depth,
        work,
    };
    body.declarations(&bc.ops)?;
    if body.suspended() {
        body.suspension_scope(&bc)
    } else if bc.ops.iter().any(|op| op.name == "LOAD_FAST_AND_CLEAR")
        && bc
            .ops
            .iter()
            .any(|op| matches!(op.name, "LIST_APPEND" | "SET_ADD" | "MAP_ADD"))
    {
        let stop = bc
            .handlers
            .first()
            .and_then(|h| bc.ops.binary_search_by_key(&h.target, |op| op.offset).ok())
            .unwrap_or(bc.ops.len());
        body.region(&bc.ops, 0, stop, 0)
    } else if bc.handlers.is_empty() {
        body.region(&bc.ops, 0, bc.ops.len(), 0)
    } else {
        body.protected(&bc)
    }
    .map_err(|e| format!("{name}: {e}"))?;
    if !body.stack.is_empty() {
        return Err(format!("{name}: unused stack values at end"));
    }
    Ok(body.lines)
}

enum Value {
    Expr(String),
    Converted(String, &'static str),
    Formatted(String),
    Interpolation(String),
    Interpolations(Vec<String>),
    Slice(String),
    InPlace(String, &'static str, String),
    Null,
    Code(Id),
    Function(Definition),
    BuildClass,
    Class(Id, Vec<String>),
    Import(Import),
    Member(String),
}

struct Body<'m, 'data, 'work> {
    m: &'m Module<'data>,
    code: &'m Code,
    stack: Vec<Value>,
    lines: String,
    loop_edges: Option<(usize, Option<usize>)>,
    depth: usize,
    class: bool,
    annotations: Vec<(Option<u32>, String, String)>,
    work: &'work mut Recovery,
}

impl Body<'_, '_, '_> {
    fn charge(&mut self, cost: usize) -> Result<(), String> {
        self.work.budget = self
            .work
            .budget
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
        match self.pop()? {
            Value::Expr(value) => Ok(value),
            Value::Formatted(field) => Ok(format!("f\"{field}\"")),
            Value::Function(def) => self.lambda(&def),
            _ => Err("expected expression".into()),
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
        } else if let Object::Slice(ids) = self.m.objects[id] {
            operands::constant_slice(self, ids)
        } else {
            let text = literal(self.m, id, 0, &mut self.work.budget)?;
            self.push(text)
        }
    }

    fn call(&mut self, argc: u32) -> Result<(), String> {
        if let Some(result) = self.special_call(argc)? {
            self.stack.push(result);
            return Ok(());
        }
        let args = self.args(argc)?;
        self.emit_call(&args)
    }

    fn emit_call(&mut self, args: &[String]) -> Result<(), String> {
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
        if matches!(self.stack.last(), Some(Value::Formatted(_))) {
            let text = self.expr()?;
            self.push(text)?;
        }
        match self.pop()? {
            Value::Expr(_) if name == "__conditional_annotations__" => Ok(()),
            Value::Function(def)
                if matches!(name, "__annotate__" | "__annotate_func__")
                    && self.m.text(self.m.code(def.id)?.name)? == "__annotate__" =>
            {
                self.annotation_scope(def.id)
            }
            Value::Expr(_)
                if self.class
                    && matches!(
                        name,
                        "__module__"
                            | "__qualname__"
                            | "__firstlineno__"
                            | "__static_attributes__"
                            | "__classdict__"
                            | "__classdictcell__"
                            | "__classcell__"
                    ) =>
            {
                Ok(())
            }
            Value::Expr(value) if name == "__doc__" && (self.depth == 0 || self.class) => {
                self.line(&value)
            }
            Value::Expr(value) => self.line(&format!("{name} = {value}")),
            Value::InPlace(lhs, operator, rhs) if lhs == name => {
                self.line(&format!("{name} {operator}= {rhs}"))
            }
            Value::Function(def) => self.definition(name, &def),
            Value::Class(id, bases) => self.class_definition(name, id, &bases),
            Value::Import(import) => self.line(&import.statement(name)?),
            Value::Member(member) => {
                let Some(Value::Import(import)) = self.stack.last_mut() else {
                    return Err("missing import".into());
                };
                import.members.push((member, name.to_owned()));
                Ok(())
            }
            _ => Err("unsupported store operand".into()),
        }
    }

    fn step(&mut self, op: Instruction) -> Result<(), String> {
        self.charge(1)?;
        let a = op.arg;
        match op.name {
            "NOP" => {
                self.work.saw_nop = true;
                if self.work.preserve_nops {
                    self.line("pass")?;
                }
            }
            "RESUME" | "NOT_TAKEN" | "COPY_FREE_VARS" | "MAKE_CELL" | "SETUP_ANNOTATIONS" => {}
            "PUSH_NULL" => self.stack.push(Value::Null),
            "LOAD_CONST" => self.constant(a)?,
            "LOAD_SMALL_INT" => self.push(a.to_string())?,
            "LOAD_COMMON_CONSTANT" => {
                let value = [
                    (CommonConstant::ASSERTIONERROR, "AssertionError"),
                    (CommonConstant::NOTIMPLEMENTEDERROR, "NotImplementedError"),
                    (CommonConstant::BUILTIN_ALL, "all"),
                    (CommonConstant::BUILTIN_ANY, "any"),
                    (CommonConstant::BUILTIN_TUPLE, "tuple"),
                    (CommonConstant::BUILTIN_LIST, "list"),
                    (CommonConstant::BUILTIN_SET, "set"),
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
            "LOAD_FAST" | "LOAD_FAST_BORROW" | "LOAD_FAST_CHECK" | "LOAD_DEREF"
            | "LOAD_CLOSURE" => {
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
            "LOAD_FROM_DICT_OR_GLOBALS" | "LOAD_FROM_DICT_OR_DEREF" => {
                if self.expr()? != "__classdict__" {
                    return Err("unsupported annotation namespace".into());
                }
                let table = if op.name == "LOAD_FROM_DICT_OR_GLOBALS" {
                    self.code.names
                } else {
                    self.code.locals
                };
                self.push(self.name(table, a)?.to_owned())?;
            }
            "LOAD_ATTR" => {
                let obj = self.expr()?;
                let attr = self.name(self.code.names, a >> 1)?;
                self.load_callable(format!("({obj}).{attr}"), a & 1 != 0)?;
            }
            "STORE_NAME" | "STORE_FAST" | "STORE_DEREF" | "STORE_GLOBAL" => {
                let tuple = if matches!(op.name, "STORE_NAME" | "STORE_GLOBAL") {
                    self.code.names
                } else {
                    self.code.locals
                };
                let name = self.name(tuple, a)?.to_owned();
                self.store(&name)?;
            }
            "POP_TOP" => {
                if let Some(Value::Import(_)) = self.stack.last() {
                    let Value::Import(import) = self.pop()? else {
                        unreachable!()
                    };
                    self.line(&import.statement("")?)?;
                } else {
                    let expr = self.expr()?;
                    self.line(&expr)?;
                }
            }
            "CALL" => self.call(a)?,
            "CALL_KW" => {
                let keys = self.expr()?;
                self.keyword_call(a, &keys)?;
            }
            "CALL_FUNCTION_EX" => self.expanded_call(a)?,
            "DICT_UPDATE" | "DICT_MERGE" if a == 1 => self.merge_map()?,
            "CALL_INTRINSIC_1"
                if a == crate::target::operands::UnaryIntrinsic::LIST_TO_TUPLE.argument() =>
            {
                let list = self.expr()?;
                let elements = list
                    .strip_prefix('[')
                    .and_then(|s| s.strip_suffix(']'))
                    .ok_or("expected argument list")?;
                self.push(format!("({elements},)"))?;
            }
            "LIST_EXTEND" | "SET_UPDATE" => self.extend_literal(op)?,
            "STORE_ATTR" | "STORE_SUBSCR" | "STORE_SLICE" | "BINARY_SLICE" | "BUILD_SLICE" => {
                self.subscript_or_attribute(op)?;
            }
            "STORE_FAST_STORE_FAST" | "STORE_FAST_LOAD_FAST" => {
                let first = self.name(self.code.locals, a >> 4)?.to_owned();
                self.store(&first)?;
                let second = self.name(self.code.locals, a & 15)?.to_owned();
                if op.name == "STORE_FAST_STORE_FAST" {
                    self.store(&second)?;
                } else {
                    self.push(second)?;
                }
            }
            "MAKE_FUNCTION" => self.make_function(a)?,
            "SET_FUNCTION_ATTRIBUTE" => self.set_attribute(a)?,
            "LOAD_BUILD_CLASS" => self.stack.push(Value::BuildClass),
            "BUILD_MAP" | "BUILD_CONST_KEY_MAP" => self.build_map(op.name, a)?,
            "IMPORT_NAME" => self.import(a)?,
            "IMPORT_FROM" => self
                .stack
                .push(Value::Member(self.name(self.code.names, a)?.to_owned())),
            "LOAD_LOCALS" if self.class => self.push("locals()".into())?,
            "BINARY_OP" | "BINARY_SUBSCR" | "COMPARE_OP" | "IS_OP" | "CONTAINS_OP" => {
                if (op.name == "BINARY_SUBSCR" || (op.name == "BINARY_OP" && a == 26))
                    && matches!(self.stack.last(), Some(Value::Slice(_)))
                {
                    let index = self.index()?;
                    let obj = self.expr()?;
                    return self.push(format!("({obj})[{index}]"));
                }
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
                    .get(if (13..26).contains(&a) {
                        (a - 13) as usize
                    } else {
                        a as usize
                    })
                    .or_else(|| (a == 26).then_some(&"[]"))
                    .ok_or("unsupported binary operator")?,
                };
                if op.name == "BINARY_OP" && (13..26).contains(&a) {
                    self.stack.push(Value::InPlace(lhs, operator, rhs));
                } else if operator == "[]" {
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
            "FORMAT_VALUE"
            | "CONVERT_VALUE"
            | "FORMAT_SIMPLE"
            | "FORMAT_WITH_SPEC"
            | "BUILD_STRING"
            | "BUILD_INTERPOLATION"
            | "BUILD_TEMPLATE" => self.string_op(op)?,
            "BUILD_TUPLE"
                if matches!(self.stack.last(), Some(Value::Interpolation(_))) && a > 0 =>
            {
                let fields = self.interpolations(a)?;
                self.stack.push(Value::Interpolations(fields));
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
                    _ => "{*()}".into(),
                };
                self.push(expr)?;
            }
            "YIELD_VALUE" => {
                let value = self.expr()?;
                self.push(format!("(yield {value})"))?;
            }
            "CALL_INTRINSIC_1"
                if a == crate::target::operands::UnaryIntrinsic::ASYNC_GEN_WRAP.argument() => {}
            "RAISE_VARARGS" => {
                let args = self.args(a)?;
                let line = match args.as_slice() {
                    [] => "raise".into(),
                    [err] => format!("raise {err}"),
                    [err, cause] => format!("raise {err} from {cause}"),
                    _ => return Err("invalid raise operand".into()),
                };
                self.line(&line)?;
            }
            "RETURN_CONST" | "RETURN_VALUE" => {
                if op.name == "RETURN_CONST" {
                    self.constant(a)?;
                }
                let expr = self.expr()?;
                if self.depth == 0 || self.class {
                    if expr != "None" {
                        return Err("module returns a value".into());
                    }
                    // CPython merges the last pass with an implicit return.
                    if self.work.preserve_nops && self.lines.ends_with("pass\n") {
                        self.line("pass")?;
                    }
                } else if expr == "None" && self.suspended() {
                    self.line("return")?;
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
        Object::Tuple(items) | Object::FrozenSet(items) => {
            let is_tuple = matches!(m.objects[id], Object::Tuple(_));
            let mut s = String::from(if is_tuple { "(" } else { "{" });
            for item in items {
                s.push_str(&literal(m, *item, depth + 1, work)?);
                s.push_str(", ");
                if s.len() > MAX_SOURCE {
                    return Err("tuple literal limit exceeded".into());
                }
            }
            s.push(if is_tuple { ')' } else { '}' });
            s
        }
        Object::Long { negative, digits } => {
            if digits.len() > MAX_SOURCE / 4 {
                return Err("integer literal limit exceeded".into());
            }
            // Marshal uses little-endian base-2^15 limbs. Hex source avoids a
            // quadratic decimal conversion and CPython's decimal digit limit.
            let mut hex = Vec::new();
            let mut buffer = 0u32;
            let mut bits = 0;
            for pair in digits.chunks_exact(2) {
                buffer |= u32::from(u16::from_le_bytes([pair[0], pair[1]])) << bits;
                bits += 15;
                while bits >= 4 {
                    hex.push(char::from_digit(buffer & 15, 16).unwrap());
                    buffer >>= 4;
                    bits -= 4;
                }
            }
            if bits != 0 {
                hex.push(char::from_digit(buffer, 16).unwrap());
            }
            while hex.last() == Some(&'0') {
                hex.pop();
            }
            if hex.is_empty() {
                "0".into()
            } else {
                let mut s = String::from(if *negative { "-0x" } else { "0x" });
                s.extend(hex.into_iter().rev());
                s
            }
        }
        _ => return Err("unsupported literal".into()),
    };
    *work = work
        .checked_sub(result.len())
        .ok_or("constant expansion limit exceeded")?;
    Ok(result)
}
