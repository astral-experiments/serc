//! Support recursive operations supplied by the pinned parser and code generator.

use ruff_python_ast::Expr;
use ruff_python_ast::visitor::{Visitor, walk_expr};

/// The maximum depth of expression nodes encountered by an AST visitor.
#[derive(Default)]
pub(crate) struct ExpressionDepth {
    current: usize,
    maximum: usize,
}

impl ExpressionDepth {
    pub(crate) fn maximum(&self) -> usize {
        self.maximum
    }
}

impl<'ast> Visitor<'ast> for ExpressionDepth {
    fn visit_expr(&mut self, expression: &'ast Expr) {
        if crate::stack_growth_required() {
            return crate::grow_stack(|| self.visit_expr(expression));
        }
        self.current += 1;
        self.maximum = self.maximum.max(self.current);
        walk_expr(self, expression);
        self.current -= 1;
    }
}

/// Whether an annotation contains only constructs with no invalid-expression diagnostics.
///
/// Walk iteratively because this is also used before invoking Ruff's recursive checker.
pub(crate) fn is_simple_annotation(expression: &Expr) -> bool {
    let mut pending = vec![expression];
    while let Some(expression) = pending.pop() {
        match expression {
            Expr::Name(_)
            | Expr::NoneLiteral(_)
            | Expr::StringLiteral(_)
            | Expr::NumberLiteral(_)
            | Expr::BooleanLiteral(_)
            | Expr::EllipsisLiteral(_) => {}
            Expr::Attribute(attribute) => pending.push(&attribute.value),
            Expr::Subscript(subscript) => {
                pending.push(&subscript.value);
                pending.push(&subscript.slice);
            }
            Expr::Call(call) if call.arguments.keywords.is_empty() => {
                pending.push(&call.func);
                pending.extend(call.arguments.args.iter());
            }
            Expr::BinOp(binary) => {
                pending.push(&binary.left);
                pending.push(&binary.right);
            }
            Expr::UnaryOp(unary) => pending.push(&unary.operand),
            _ => return false,
        }
    }
    true
}

/// Reserve a stack for a third-party recursive walk based on its input depth.
///
/// The parser and code generator do not expose hooks for segmented-stack growth.
pub(crate) fn with_depth<T>(depth: usize, callback: impl FnOnce() -> T) -> T {
    // Debug builds have substantially larger generated visitor frames.
    let bytes_per_level: usize = if cfg!(debug_assertions) {
        8 * 1024
    } else {
        2 * 1024
    };
    let required = depth.saturating_add(16).saturating_mul(bytes_per_level);
    stacker::maybe_grow(required, required.max(crate::STACK_SEGMENT_SIZE), callback)
}

#[cfg(test)]
mod tests {
    #[test]
    fn fallback_stack_growth_makes_progress_and_restores_state() {
        fn walk(depth: usize) -> usize {
            if crate::stack_growth_required_for(None) {
                return crate::with_stack_reentry_bypass(|| walk(depth));
            }
            if depth == 0 { 1 } else { 1 + walk(depth - 1) }
        }

        assert_eq!(walk(50), 51);
        crate::with_stack_reentry_bypass(|| {
            assert!(!crate::stack_growth_required_for(Some(
                crate::STACK_RED_ZONE
            )));
            assert!(crate::stack_growth_required_for(None));
        });
        let result = std::panic::catch_unwind(|| {
            crate::with_stack_reentry_bypass(|| {
                assert!(!crate::stack_growth_required_for(None));
                panic!("simulate a failed operation");
            });
        });
        assert!(result.is_err());
        assert!(crate::stack_growth_required_for(None));
    }
}
