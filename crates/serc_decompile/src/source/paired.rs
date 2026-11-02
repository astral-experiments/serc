//! Pair recompiled scopes within their parent in constant-table order.
use std::collections::BTreeMap;

use rustc_hash::FxHashSet;

use super::{Id, MAX_WORK, Module, Object};

fn children<'a>(
    m: &'a Module<'_>,
    id: Id,
    budget: &mut usize,
) -> Option<BTreeMap<&'a str, Vec<Id>>> {
    let constants = m.tuple(m.code(id).ok()?.constants).ok()?;
    *budget = budget.checked_sub(1 + constants.len())?;
    let mut groups = BTreeMap::<_, Vec<_>>::new();
    for &child in constants {
        if let Object::Code(code) = &m.objects[child] {
            let name = m.text(code.qualname).ok()?;
            *budget = budget.checked_sub(name.len())?;
            groups.entry(name).or_default().push(child);
        }
    }
    Some(groups)
}

/// Skip repeated names when a definition was lost and skip shared pairs already visited.
pub(super) fn visit(
    m: &Module<'_>,
    candidate: &Module<'_>,
    mut visit: impl FnMut(Id, Id, &mut usize) -> Option<()>,
) -> Option<()> {
    let mut pending = vec![(m.root, candidate.root)];
    let mut seen = FxHashSet::default();
    let mut budget = MAX_WORK;
    while let Some((id, other)) = pending.pop() {
        if !seen.insert((id, other)) {
            continue;
        }
        visit(id, other, &mut budget)?;
        let mut peers = children(candidate, other, &mut budget)?;
        for (name, ids) in children(m, id, &mut budget)? {
            if let Some(others) = peers.remove(name)
                && ids.len() == others.len()
            {
                pending.extend(ids.into_iter().zip(others));
            }
        }
    }
    Some(())
}
