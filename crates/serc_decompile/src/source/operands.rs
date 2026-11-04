//! Source operands shared across old and new instruction layouts.
use super::{
    Body, Instruction, Value,
    definitions::{string_value, tuple_items},
    identifier,
};
use serc_bytecode::target::operands::BinaryOperation;

fn slice_text(items: &[String]) -> String {
    items
        .iter()
        .map(|s| if s == "None" { "" } else { s })
        .collect::<Vec<_>>()
        .join(":")
}

/// CPython saves the receiver and indices once before loading an in-place target.
pub(super) fn augmented_load(ops: &[Instruction]) -> Option<usize> {
    let n = ops.first()?.arg as usize;
    if !(1..=3).contains(&n)
        || !ops
            .get(..n)?
            .iter()
            .all(|op| op.name == "COPY" && op.arg as usize == n)
    {
        return None;
    }
    let load = ops.get(n)?;
    match (n, load.name) {
        (1, "LOAD_ATTR") if load.arg & 1 == 0 => Some(n),
        (2, "BINARY_SUBSCR") | (3, "BINARY_SLICE") => Some(n),
        (2, "BINARY_OP") if load.arg == BinaryOperation::SUBSCR.argument() => Some(n),
        _ => None,
    }
}

/// The optimizer can reverse fast-local stores and eliminate the preceding
/// SWAP. A fused load belongs to the next expression, after the last store.
pub(super) fn local_store_run(ops: &[Instruction], count: usize) -> bool {
    if !(2..=3).contains(&count) {
        return false;
    }
    let mut left = count;
    for op in ops {
        let stores = match op.name {
            "STORE_FAST" => 1,
            "STORE_FAST_STORE_FAST" => 2,
            "STORE_FAST_LOAD_FAST" => return left == 1,
            _ => return false,
        };
        if stores >= left {
            return stores == left;
        }
        left -= stores;
    }
    false
}

pub(super) struct Binding {
    pub(super) target: String,
    pub(super) next: usize,
    pub(super) seed: Vec<Value>,
}

impl Body<'_, '_, '_> {
    pub(super) fn augmented_store(
        &mut self,
        ops: &[Instruction],
        load: usize,
        count: usize,
        end: usize,
        level: usize,
    ) -> Result<usize, String> {
        self.step(ops[load])?;
        let stores = ["STORE_ATTR", "STORE_SUBSCR", "STORE_SLICE"];
        let inplace =
            BinaryOperation::INPLACE_ADD.argument()..=BinaryOperation::INPLACE_XOR.argument();
        for binary in load + 1..end {
            self.charge(1)?;
            let operation = ops[binary];
            if operation.name != "BINARY_OP" || !inplace.contains(&operation.arg) {
                continue;
            }
            let store = binary + 1 + count;
            if store >= end
                || ops[store].name != stores[count - 1]
                || (count == 1 && ops[store].arg != ops[load].arg >> 1)
                || !(0..count).all(|i| {
                    ops[binary + 1 + i].name == "SWAP"
                        && ops[binary + 1 + i].arg as usize == count + 1 - i
                })
            {
                continue;
            }
            let rhs = self.fragment(ops, load + 1, binary, level)?.expression()?;
            self.push(rhs)?;
            self.step(operation)?;
            let Value::InPlace(target, operator, value) = self.pop()? else {
                return Err("expected in-place operation".into());
            };
            self.line(&format!("{target} {operator}= {value}"))?;
            return Ok(store + 1);
        }
        Err("missing in-place target store".into())
    }

    pub(super) fn append_dict_item(&mut self, item: &str) -> Result<(), String> {
        let map = self.expr()?;
        let prefix = map
            .strip_prefix('{')
            .and_then(|s| s.strip_suffix('}'))
            .ok_or("unsupported map update")?;
        let separator = if prefix.is_empty() { "" } else { ", " };
        self.push(format!("{{{prefix}{separator}{item}}}"))
    }

    pub(super) fn expanded_call(&mut self, flag: u32) -> Result<(), String> {
        let keywords = if self.m.version >= serc_bytecode::PythonVersion::Py314 {
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
        self.call(argc, &tuple_items(keys)?)
    }

    pub(super) fn call_args(&mut self, argc: u32, keys: &[String]) -> Result<Vec<String>, String> {
        let mut args = self.args(argc)?;
        let first = args
            .len()
            .checked_sub(keys.len())
            .ok_or("invalid keyword argument count")?;
        for (arg, key) in args[first..].iter_mut().zip(keys) {
            let key = string_value(key)?;
            *arg = format!("{}={arg}", identifier(&key)?);
        }
        Ok(args)
    }

    pub(super) fn update_literal(&mut self, op: Instruction) -> Result<(), String> {
        if op.arg != 1 {
            return Err("unsupported collection update".into());
        }
        let items = self.expr()?;
        let collection = self.expr()?;
        let (open, close) = if matches!(op.name, "LIST_EXTEND" | "LIST_APPEND") {
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
        let body = if matches!(op.name, "LIST_APPEND" | "SET_ADD") {
            items
        } else {
            // A tuple used by LIST_EXTEND or SET_UPDATE is still one iterable,
            // even when its elements could be written as separate operands.
            format!("*({items})")
        };
        self.push(format!("{open}{prefix}{body}{close}"))
    }

    /// Decode the stores for one Python target, keeping any load fused with the
    /// final store as the first operand of the following expression or suite.
    pub(super) fn binding(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
    ) -> Result<Binding, String> {
        self.bindings(ops, start, end, 1, false)
    }

    pub(super) fn parallel_assignment(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        count: usize,
        reversed: bool,
    ) -> Result<usize, String> {
        let rhs = self.args(u32::try_from(count).map_err(|_| "invalid assignment arity")?)?;
        let binding = self.bindings(ops, start, end, count, reversed)?;
        self.line(&format!("{} = ({})", binding.target, rhs.join(", ")))?;
        self.stack.extend(binding.seed);
        Ok(binding.next)
    }

    /// Each COPY belongs to one more target of a single assignment. Read a
    /// final target before consuming the RHS: an isolated copy and store can
    /// instead be a walrus inside an expression.
    pub(super) fn chained_targets(
        &mut self,
        ops: &[Instruction],
        mut next: usize,
    ) -> Result<Binding, String> {
        let mut pending = None;
        let mut target = String::new();
        loop {
            self.write_target(ops, &mut next, &mut pending, &mut target, 0)?;
            self.charge(3)?;
            target.push_str(" = ");
            if pending.is_some()
                || ops
                    .get(next)
                    .is_none_or(|op| (op.name, op.arg) != ("COPY", 1))
            {
                break;
            }
            next += 1;
        }
        self.write_target(ops, &mut next, &mut pending, &mut target, 0)?;
        let seed = self.binding_seed(pending)?;
        Ok(Binding { target, next, seed })
    }

    fn bindings(
        &mut self,
        ops: &[Instruction],
        start: usize,
        end: usize,
        count: usize,
        reversed: bool,
    ) -> Result<Binding, String> {
        let mut next = start;
        let mut pending = None;
        let mut targets = Vec::new();
        for _ in 0..count {
            let mut target = String::new();
            self.write_target(&ops[..end], &mut next, &mut pending, &mut target, 0)?;
            targets.push(target);
        }
        if reversed {
            targets.reverse();
        }
        let target = targets.join(", ");
        let seed = self.binding_seed(pending)?;
        Ok(Binding { target, next, seed })
    }

    fn binding_seed(&self, pending: Option<(&str, u32)>) -> Result<Vec<Value>, String> {
        match pending {
            Some(("LOAD_FAST", index)) => Ok(vec![Value::Expr(
                self.name(self.code.locals, index)?.to_owned(),
            )]),
            None => Ok(Vec::new()),
            _ => Err("unconsumed store after unpacking target".into()),
        }
    }

    fn write_target(
        &mut self,
        ops: &[Instruction],
        next: &mut usize,
        pending: &mut Option<(&'static str, u32)>,
        text: &mut String,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 32 || text.len() >= super::MAX_SOURCE {
            return Err("unpacking target limit exceeded".into());
        }
        self.charge(1)?;
        if let Some(("LOAD_FAST", index)) = *pending {
            *pending = None;
            let receiver = Value::Expr(self.name(self.code.locals, index)?.to_owned());
            let (target, after) = self.computed_target(ops, *next, vec![receiver])?;
            text.push_str(&target);
            *next = after;
            return Ok(());
        }
        let (name, arg) = if let Some(op) = pending.take() {
            op
        } else {
            let op = ops.get(*next).ok_or("incomplete unpacking target")?;
            *next += 1;
            (op.name, op.arg)
        };
        if matches!(name, "UNPACK_SEQUENCE" | "UNPACK_EX") {
            let star = (name == "UNPACK_EX").then_some(arg & 255);
            let count = star.map_or(arg, |before| before + 1 + (arg >> 8));
            text.push('(');
            for i in 0..count {
                if i > 0 {
                    text.push_str(", ");
                }
                if star == Some(i) {
                    text.push('*');
                }
                self.write_target(ops, next, pending, text, depth + 1)?;
            }
            if count == 1 {
                text.push(',');
            }
            text.push(')');
            return Ok(());
        }
        let (table, index) = match name {
            "STORE_NAME" | "STORE_GLOBAL" => (self.code.names, arg),
            "STORE_FAST" | "STORE_DEREF" => (self.code.locals, arg),
            "STORE_FAST_STORE_FAST" | "STORE_FAST_LOAD_FAST" => {
                let second = if name == "STORE_FAST_LOAD_FAST" {
                    "LOAD_FAST"
                } else {
                    "STORE_FAST"
                };
                *pending = Some((second, arg & 15));
                (self.code.locals, arg >> 4)
            }
            _ if name.starts_with("LOAD_")
                || matches!(name, "PUSH_NULL" | "BUILD_LIST" | "BUILD_MAP") =>
            {
                let start = *next - 1;
                let (target, after) = self.computed_target(ops, start, Vec::new())?;
                text.push_str(&target);
                *next = after;
                return Ok(());
            }
            _ => return Err("unsupported unpacking target".into()),
        };
        let name = self.name(table, index)?;
        text.push_str(name);
        self.charge(name.len())
    }

    // The receiver and indices are evaluated after the RHS. The store is the
    // first instruction that consumes the value beneath these operands.
    fn computed_target(
        &mut self,
        ops: &[Instruction],
        start: usize,
        seed: Vec<Value>,
    ) -> Result<(String, usize), String> {
        if start >= ops.len() {
            return Err("incomplete assignment target".into());
        }
        let end = ops.len().min(self.flow.block(start).ops.end);
        let mut height = seed.len();
        for i in start..end {
            let op = ops[i];
            self.charge(1)?;
            let (pops, pushes) =
                serc_bytecode::target::stack_counts_for(self.m.version, op.opcode, op.arg);
            if pops > height {
                if pops != height + 1
                    || !matches!(op.name, "STORE_ATTR" | "STORE_SUBSCR" | "STORE_SLICE")
                {
                    break;
                }
                let values = self.seeded(ops, start, i, 0, seed)?.operands()?;
                let target = match (op.name, values.as_slice()) {
                    ("STORE_ATTR", [Value::Expr(obj)]) => {
                        format!("({obj}).{}", self.name(self.code.names, op.arg)?)
                    }
                    (
                        "STORE_SUBSCR",
                        [Value::Expr(obj), Value::Expr(index) | Value::Slice(index)],
                    ) => {
                        // An eager annotation stores its expression here. Leave a
                        // preceding walrus on the stack for annotation recovery.
                        if self.m.version <= serc_bytecode::PythonVersion::Py313
                            && obj == "__annotations__"
                            && string_value(index).is_ok()
                        {
                            break;
                        }
                        format!("({obj})[{index}]")
                    }
                    ("STORE_SLICE", [Value::Expr(obj), Value::Expr(lower), Value::Expr(upper)]) => {
                        let lower = if lower == "None" { "" } else { lower };
                        let upper = if upper == "None" { "" } else { upper };
                        format!("({obj})[{lower}:{upper}]")
                    }
                    _ => break,
                };
                return Ok((target, i + 1));
            }
            if op.jump.is_some()
                || op.name.starts_with("STORE_")
                || matches!(op.name, "COPY" | "NOP")
            {
                break;
            }
            height = height - pops + pushes;
        }
        Err("unsupported unpacking target".into())
    }

    pub(super) fn push_slice_tuple(&mut self, items: &[String]) -> Result<(), String> {
        let mut text = items.join(", ");
        if items.len() == 1 {
            text.push(',');
        }
        self.charge(text.len() + 1)?;
        if text.len() > super::MAX_SOURCE {
            return Err("slice index size limit exceeded".into());
        }
        self.stack.push(Value::Slice(text));
        Ok(())
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
            let annotation = if self.code.flags
                & serc_bytecode::target::code_flags::CO_FUTURE_ANNOTATIONS
                != 0
            {
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
) -> Result<String, String> {
    let mut items = Vec::new();
    for id in ids {
        items.push(super::literal(body.m, id, 0, &mut body.work.budget)?);
    }
    if items[2] == "None" {
        items.pop();
    }
    Ok(slice_text(&items))
}
