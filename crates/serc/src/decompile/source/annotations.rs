//! Module and class annotations live in a separate function on 3.14+.
//! Conditional annotations carry a numbered marker in the enclosing scope.
use super::{Body, Id, identifier};

impl Body<'_, '_, '_> {
    pub(super) fn annotation_scope(&mut self, id: Id) -> Result<(), String> {
        let (mut helper, bc) = self.wrapper(id)?;
        let ops = &bc.ops;
        let mut start = ops
            .iter()
            .position(|op| op.name == "BUILD_MAP" && op.arg == 0)
            .ok_or("missing annotation map")?
            + 1;
        let mut entries = Vec::new();
        while ops.get(start).is_some_and(|op| op.name != "RETURN_VALUE") {
            let copy = ops[start..]
                .iter()
                .position(|op| op.name == "COPY" && op.arg == 2)
                .map(|n| start + n)
                .ok_or("unsupported annotation map write")?;
            let mut marker = None;
            if let Some(test) = ops[start..copy]
                .iter()
                .position(|op| op.name == "CONTAINS_OP")
                .map(|n| start + n)
            {
                let collection = test
                    .checked_sub(1)
                    .filter(|&i| i > start)
                    .ok_or("invalid annotation guard")?;
                let number = helper
                    .fragment(ops, start, collection, 0)?
                    .expression()?
                    .parse::<u32>()
                    .map_err(|_| "invalid annotation marker")?;
                marker = Some(number);
                start = test + 2;
                if ops.get(start).is_some_and(|op| op.name == "NOT_TAKEN") {
                    start += 1;
                }
            }
            let expr = helper.fragment(ops, start, copy, 0)?.expression()?;
            let key = ops
                .get(copy + 1)
                .filter(|op| op.name == "LOAD_CONST")
                .ok_or("unsupported annotation binding")?;
            if ops.get(copy + 2).is_none_or(|op| op.name != "STORE_SUBSCR") {
                return Err("unsupported annotation store".into());
            }
            let name = identifier(
                helper
                    .m
                    .text(helper.item(helper.code.constants, key.arg)?)?,
            )?;
            entries.push((marker, name.to_owned(), expr));
            start = copy + 3;
        }
        for (marker, name, value) in entries {
            if marker.is_some() {
                self.annotations.push((marker, name, value));
            } else {
                self.annotate(&name, &value)?;
            }
        }
        Ok(())
    }

    pub(super) fn annotation_site(&mut self) -> Result<(), String> {
        self.charge(self.annotations.len())?;
        let marker = self
            .expr()?
            .parse::<u32>()
            .map_err(|_| "invalid annotation index")?;
        if self.expr()? != "__conditional_annotations__" {
            return Err("unsupported annotation marker".into());
        }
        let index = self
            .annotations
            .iter()
            .position(|(n, _, _)| *n == Some(marker))
            .ok_or("missing conditional annotation")?;
        let (_, name, value) = self.annotations.remove(index);
        self.annotate(&name, &value)
    }

    pub(super) fn annotate(&mut self, name: &str, annotation: &str) -> Result<(), String> {
        self.charge(self.lines.len())?;
        let prefix = format!("{name} = ");
        if let Some(index) = self
            .lines
            .rmatch_indices(&prefix)
            .find_map(|(i, _)| (i == 0 || self.lines.as_bytes()[i - 1] == b'\n').then_some(i))
        {
            self.charge(annotation.len() + 2)?;
            if self.lines.len() + annotation.len() + 2 >= super::MAX_SOURCE {
                return Err("source size limit exceeded".into());
            }
            self.lines
                .insert_str(index + name.len(), &format!(": {annotation}"));
            Ok(())
        } else {
            self.line(&format!("{name}: {annotation}"))
        }
    }
}
