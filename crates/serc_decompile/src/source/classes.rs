//! Class construction has two implicit arguments: the body function and name.
//! Generic classes also insert a compiler-created base between the positional
//! bases and any class keywords.
use serc_bytecode::target::operands::UnaryIntrinsic;

use super::{
    Body, Instruction, Object, Value,
    definitions::{Definition, tuple_items},
};

impl Body<'_, '_, '_> {
    pub(super) fn super_attribute(&mut self, arg: u32) -> Result<(), String> {
        let instance = self.expr()?;
        let class = self.expr()?;
        if self.expr()? != "super" {
            return Err("expected super callable".into());
        }
        let args = if arg & 2 != 0 {
            format!("{class}, {instance}")
        } else {
            if class != "__class__" {
                return Err("invalid implicit super class".into());
            }
            String::new()
        };
        let attr = self.name(self.code.names, arg >> 2)?;
        self.load_callable(format!("super({args}).{attr}"), arg & 1 != 0)
    }

    pub(super) fn class_parameters(&self, ops: &[Instruction]) -> Result<bool, String> {
        let [_, load, store, ..] = ops else {
            return Ok(false);
        };
        Ok(load.name == "LOAD_FROM_DICT_OR_DEREF"
            && store.name == "STORE_NAME"
            && self.m.text(self.item(self.code.locals, load.arg)?)? == ".type_params"
            && self.name(self.code.names, store.arg)? == "__type_params__")
    }

    pub(super) fn generic_class(
        &mut self,
        ops: &[Instruction],
        store: usize,
        params: Vec<String>,
    ) -> Result<Value, String> {
        let hidden = self.m.text(self.item(self.code.locals, ops[store].arg)?)?;
        if hidden != ".type_params" {
            return Err("expected class type parameter cell".into());
        }
        let generic = ops[store + 1..]
            .iter()
            .position(|op| {
                op.name == "CALL_INTRINSIC_1"
                    && op.arg == UnaryIntrinsic::SUBSCRIPT_GENERIC.argument()
            })
            .map(|i| store + 1 + i)
            .ok_or("missing implicit generic base")?;
        let class = ops[store + 1..generic]
            .iter()
            .find_map(|op| {
                if op.name != "LOAD_CONST" {
                    return None;
                }
                let id = self.item(self.code.constants, op.arg).ok()?;
                matches!(self.m.objects[id], Object::Code(_)).then_some(id)
            })
            .ok_or("missing generic class body")?;
        let [call, ret] = ops.last_chunk::<2>().ok_or("missing generic class call")?;
        if !matches!(call.name, "CALL" | "CALL_KW") || ret.name != "RETURN_VALUE" {
            return Err("unsupported generic class call".into());
        }
        let mut end = ops.len() - 2;
        let keys = if end > generic && (call.name == "CALL_KW" || ops[end - 1].name == "KW_NAMES") {
            end -= 1;
            if !matches!(ops[end].name, "KW_NAMES" | "LOAD_CONST") {
                return Err("expected class keyword names".into());
            }
            self.constant(ops[end].arg)?;
            tuple_items(&self.expr()?)?
        } else {
            Vec::new()
        };
        let save = *ops.get(generic + 1).ok_or("missing generic base binding")?;
        let fused = save.name == "STORE_FAST_LOAD_FAST";
        let index = if fused { save.arg >> 4 } else { save.arg };
        if !(fused || save.name == "STORE_FAST")
            || self.m.text(self.item(self.code.locals, index)?)? != ".generic_base"
        {
            return Err("invalid generic base binding".into());
        }
        let start = generic + 2;
        if start > end {
            return Err("invalid generic class argument range".into());
        }
        let mut implicit = fused && save.arg & 15 == index;
        if fused && !implicit {
            self.push(self.name(self.code.locals, save.arg & 15)?.to_owned())?;
        }
        let mut segment = start;
        for i in start..end {
            let op = ops[i];
            let pair = matches!(
                op.name,
                "LOAD_FAST_LOAD_FAST" | "LOAD_FAST_BORROW_LOAD_FAST_BORROW"
            );
            if pair && [op.arg >> 4, op.arg & 15].contains(&index)
                || matches!(op.name, "LOAD_FAST" | "LOAD_FAST_BORROW") && op.arg == index
            {
                if implicit {
                    return Err("repeated implicit generic base".into());
                }
                self.region(ops, segment, i, 0)?;
                if pair {
                    for arg in [op.arg >> 4, op.arg & 15] {
                        if arg != index {
                            self.push(self.name(self.code.locals, arg)?.to_owned())?;
                        }
                    }
                }
                implicit = true;
                segment = i + 1;
            }
        }
        if !implicit {
            return Err("missing implicit generic base argument".into());
        }
        self.region(ops, segment, end, 0)?;
        let bases = self.call_args(
            call.arg
                .checked_sub(3)
                .ok_or("missing generic class arguments")?,
            &keys,
        )?;
        let mut def = Definition::new(class);
        def.type_params = params;
        Ok(Value::Class(def, bases))
    }
}
