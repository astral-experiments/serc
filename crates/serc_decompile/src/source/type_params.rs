//! PEP 695 declarations. Each bound, default and alias value is evaluated in
//! its own compiler-generated scope, which may close over earlier parameters.
use serc_bytecode::PythonVersion;
use serc_bytecode::target::operands::{BinaryIntrinsic, UnaryIntrinsic};

use super::{
    Body, Id, Instruction, Value, definitions::Definition, definitions::string_value, identifier,
};

pub(super) fn is_alias_intrinsic(op: &Instruction) -> bool {
    op.name == "CALL_INTRINSIC_1" && op.arg == UnaryIntrinsic::TYPEALIAS.argument()
}

impl Body<'_, '_, '_> {
    pub(super) fn make_alias(&mut self) -> Result<(), String> {
        let Value::Function(mut def) = self.pop()? else {
            return Err("expected type alias value function".into());
        };
        match self.pop()? {
            Value::TypeParams(params) => def.type_params = params,
            Value::Expr(value) if value == "None" => {}
            _ => return Err("invalid type alias parameters".into()),
        }
        if string_value(&self.expr()?)? != self.m.text(self.m.code(def.id)?.name)? {
            return Err("invalid type alias operands".into());
        }
        self.stack.push(Value::TypeAlias(def));
        Ok(())
    }

    pub(super) fn alias_definition(&mut self, name: &str, def: &Definition) -> Result<(), String> {
        if name != self.m.text(self.m.code(def.id)?.name)? {
            return Err("type alias binding differs from code name".into());
        }
        let rhs = self.lazy_value(def.id, false)?;
        let params = if def.type_params.is_empty() {
            String::new()
        } else {
            format!("[{}]", def.type_params.join(", "))
        };
        self.line(&format!("type {name}{params} = {rhs}"))
    }

    pub(super) fn lazy_value(&mut self, id: Id, unpack: bool) -> Result<String, String> {
        let (mut body, bc) = self.wrapper(id)?;
        let (ret, ops) = bc.ops.split_last().ok_or("empty lazy evaluation scope")?;
        if !matches!(ret.name, "RETURN_CONST" | "RETURN_VALUE") {
            return Err("unsupported lazy evaluation return".into());
        }
        let start = if body.m.version >= PythonVersion::Py314 {
            let guard = ops
                .iter()
                .position(|op| op.name == "POP_JUMP_IF_FALSE")
                .ok_or("missing format guard")?;
            let target = body.flow.target(guard)?;
            if target <= guard
                || ops
                    .get(target - 1)
                    .is_none_or(|op| op.name != "RAISE_VARARGS")
            {
                return Err("unsupported format guard".into());
            }
            target
        } else {
            0
        };
        let star = unpack
            && ops
                .last()
                .is_some_and(|op| (op.name, op.arg) == ("UNPACK_SEQUENCE", 1));
        body.region(&bc.ops, start, ops.len() - usize::from(star), 0)?;
        if ret.name == "RETURN_CONST" {
            body.constant(ret.arg)?;
        }
        let value = body.expr()?;
        if !body.stack.is_empty() || !body.lines.is_empty() {
            return Err("lazy scope is not a single expression".into());
        }
        Ok(if star { format!("*{value}") } else { value })
    }

    fn parameter_value(
        &mut self,
        ops: &[Instruction],
        start: usize,
        stop: usize,
        unpack: bool,
    ) -> Result<String, String> {
        self.region(ops, start, stop, 0)?;
        let Value::Function(def) = self.pop()? else {
            return Err("expected type parameter evaluation function".into());
        };
        self.lazy_value(def.id, unpack)
    }

    fn type_parameter(&mut self, ops: &[Instruction], pos: &mut usize) -> Result<String, String> {
        let load = ops
            .get(*pos)
            .filter(|op| op.name == "LOAD_CONST")
            .ok_or("missing type parameter name")?;
        let name = identifier(self.m.text(self.item(self.code.constants, load.arg)?)?)?.to_owned();
        let start = *pos + 1;
        let intrinsic = ops[start..]
            .iter()
            .position(|op| matches!(op.name, "CALL_INTRINSIC_1" | "CALL_INTRINSIC_2"))
            .map(|n| start + n)
            .ok_or("missing type parameter intrinsic")?;
        let op = ops[intrinsic];
        let mut text = if op.name == "CALL_INTRINSIC_2"
            && [
                BinaryIntrinsic::TYPEVAR_WITH_BOUND.argument(),
                BinaryIntrinsic::TYPEVAR_WITH_CONSTRAINTS.argument(),
            ]
            .contains(&op.arg)
        {
            format!(
                "{name}: {}",
                self.parameter_value(ops, start, intrinsic, false)?
            )
        } else {
            if intrinsic != start || op.name != "CALL_INTRINSIC_1" {
                return Err("unsupported type parameter layout".into());
            }
            let prefix = if op.arg == UnaryIntrinsic::TYPEVAR.argument() {
                ""
            } else if op.arg == UnaryIntrinsic::PARAMSPEC.argument() {
                "**"
            } else if op.arg == UnaryIntrinsic::TYPEVARTUPLE.argument() {
                "*"
            } else {
                return Err("unsupported type parameter intrinsic".into());
            };
            format!("{prefix}{name}")
        };
        let next = intrinsic + 1;
        let copy = ops[next..]
            .iter()
            .position(|op| (op.name, op.arg) == ("COPY", 1))
            .map(|n| next + n)
            .ok_or("missing type parameter binding")?;
        if copy > next {
            let default = ops[copy - 1];
            if default.name != "CALL_INTRINSIC_2"
                || default.arg != BinaryIntrinsic::SET_TYPEPARAM_DEFAULT.argument()
            {
                return Err("unsupported type parameter default".into());
            }
            let value = self.parameter_value(
                ops,
                next,
                copy - 1,
                op.name == "CALL_INTRINSIC_1" && op.arg == UnaryIntrinsic::TYPEVARTUPLE.argument(),
            )?;
            text.push_str(" = ");
            text.push_str(&value);
        }
        let store = ops.get(copy + 1).ok_or("missing type parameter store")?;
        if !matches!(store.name, "STORE_FAST" | "STORE_DEREF")
            || self.name(self.code.locals, store.arg)? != name
        {
            return Err("unsupported type parameter binding".into());
        }
        *pos = copy + 2;
        Ok(text)
    }

    pub(super) fn generic(&mut self, id: Id) -> Result<Value, String> {
        let (mut body, bc) = self.wrapper(id)?;
        let ops = &bc.ops;
        let mut pos = ops
            .iter()
            .position(|op| op.name == "LOAD_CONST")
            .ok_or("missing type parameters")?;
        let alias = ops.iter().rev().nth(1).is_some_and(is_alias_intrinsic);
        if alias {
            body.constant(ops[pos].arg)?;
            pos += 1;
        }
        let mut params = Vec::new();
        while ops.get(pos).is_some_and(|op| op.name != "BUILD_TUPLE") {
            params.push(body.type_parameter(ops, &mut pos)?);
        }
        if params.is_empty()
            || ops
                .get(pos)
                .is_none_or(|op| op.arg as usize != params.len())
        {
            return Err("unsupported generic parameter tuple".into());
        }
        if alias {
            body.stack.push(Value::TypeParams(params));
            body.region(ops, pos + 1, ops.len() - 1, 0)?;
            return body.pop();
        }
        let swap = ops
            .iter()
            .rposition(|op| op.name == "SWAP")
            .ok_or("unsupported generic result")?;
        body.region(ops, pos + 1, swap, 0)?;
        let Value::Function(mut def) = body.pop()? else {
            return Err("expected generic function".into());
        };
        def.type_params = params;
        Ok(Value::Function(def))
    }
}
