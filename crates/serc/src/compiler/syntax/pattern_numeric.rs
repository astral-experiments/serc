//! Exact numeric comparisons for large integer mapping-pattern keys.

use ruff_python_ast::visitor::{Visitor, walk_pattern};
use ruff_python_ast::{self as ast, Expr, Pattern};
use rustc_hash::FxHashSet;

use crate::PythonVersion;

use super::super::constant::{
    PythonNumericValueKey, fold_constant_with_literals, literal_constant, python_numeric_value_key,
};

/// Index numeric keys without rounding integers through a float.
#[derive(Default)]
pub(super) struct LargeNumericKeys {
    numbers: FxHashSet<PythonNumericValueKey>,
    large_numbers: FxHashSet<PythonNumericValueKey>,
}

impl LargeNumericKeys {
    pub(super) fn duplicate(&mut self, key: &Expr, version: ast::PythonVersion) -> bool {
        if !matches!(
            key,
            Expr::NumberLiteral(_) | Expr::BooleanLiteral(_) | Expr::UnaryOp(_) | Expr::BinOp(_)
        ) {
            return false;
        }
        let version = match version {
            ast::PythonVersion::PY312 => PythonVersion::Py312,
            ast::PythonVersion::PY313 => PythonVersion::Py313,
            ast::PythonVersion::PY314 => PythonVersion::Py314,
            _ => PythonVersion::Py315,
        };
        let Some(constant) = fold_constant_with_literals(key, version, &literal_constant, false)
        else {
            return false;
        };
        let Some(number) = python_numeric_value_key(&constant) else {
            return false;
        };
        let large = large_integer_literal(key);
        let duplicate =
            (large && self.numbers.contains(&number)) || self.large_numbers.contains(&number);
        self.numbers.insert(number.clone());
        if large {
            self.large_numbers.insert(number);
        }
        duplicate
    }
}

/// Only a literal integer outside Ruff's supported range needs the ordered retry.
fn large_integer_literal(key: &Expr) -> bool {
    if crate::stack_growth_required() {
        return crate::grow_stack(|| large_integer_literal(key));
    }
    match key {
        Expr::NumberLiteral(number) => {
            matches!(&number.value, ast::Number::Int(integer) if integer.as_u64().is_none())
        }
        Expr::UnaryOp(unary) if unary.op == ast::UnaryOp::USub => {
            large_integer_literal(&unary.operand)
        }
        Expr::BinOp(binary) => {
            large_integer_literal(&binary.left) || large_integer_literal(&binary.right)
        }
        _ => false,
    }
}

/// An ordered retry is needed only when the shared checker can miss a duplicate.
pub(super) fn has_large_numeric_duplicate(
    statement: &ast::StmtMatch,
    version: ast::PythonVersion,
) -> bool {
    struct Finder(bool, ast::PythonVersion);
    impl<'ast> Visitor<'ast> for Finder {
        fn visit_pattern(&mut self, pattern: &'ast Pattern) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_pattern(pattern));
            }
            if self.0 {
                return;
            }
            if let Pattern::MatchMapping(mapping) = pattern
                && mapping.keys.len() > 1
                && mapping.keys.iter().any(large_integer_literal)
            {
                let mut seen = LargeNumericKeys::default();
                if mapping.keys.iter().any(|key| seen.duplicate(key, self.1)) {
                    self.0 = true;
                    return;
                }
            }
            walk_pattern(self, pattern);
        }
        fn visit_expr(&mut self, _expression: &'ast Expr) {}
    }
    let mut finder = Finder(false, version);
    for case in &statement.cases {
        finder.visit_pattern(&case.pattern);
        if finder.0 {
            break;
        }
    }
    finder.0
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::super::validate;
    use crate::code_object::Constant;
    use crate::compiler::constant::{python_constant_values_equal, python_numeric_value_key};
    use crate::{CompileError, PythonVersion};

    const TARGETS: [PythonVersion; 4] = [
        PythonVersion::Py312,
        PythonVersion::Py313,
        PythonVersion::Py314,
        PythonVersion::Py315,
    ];

    #[test]
    fn numeric_hash_keys_match_python_equality() {
        let values = [
            Constant::Bool(false),
            Constant::Int(0),
            Constant::Float(-0.0),
            Constant::Complex {
                real: 0.0,
                imag: -0.0,
            },
            Constant::Bool(true),
            Constant::Int(1),
            Constant::Float(1.0),
            Constant::Float(1.5),
            Constant::Complex {
                real: 1.0,
                imag: -0.0,
            },
            Constant::Complex {
                real: 1.0,
                imag: 1.5,
            },
            Constant::BigInt {
                negative: false,
                digits: vec![0, 0, 0, 0, 16],
            },
            Constant::BigInt {
                negative: false,
                digits: vec![1, 0, 0, 0, 16],
            },
            Constant::Float(18_446_744_073_709_551_616.0),
            Constant::Complex {
                real: 18_446_744_073_709_551_616.0,
                imag: 0.0,
            },
            Constant::Complex {
                real: 18_446_744_073_709_551_616.0,
                imag: 1.0,
            },
            Constant::Float(f64::INFINITY),
            Constant::Float(f64::NEG_INFINITY),
            Constant::Complex {
                real: 1.0,
                imag: f64::INFINITY,
            },
        ];
        for left in &values {
            for right in &values {
                let left_key = python_numeric_value_key(left).unwrap();
                let right_key = python_numeric_value_key(right).unwrap();
                assert_eq!(
                    left_key == right_key,
                    python_constant_values_equal(left, right),
                    "{left:?} vs {right:?}"
                );
            }
        }
        assert!(python_numeric_value_key(&Constant::Float(f64::NAN)).is_none());
        assert!(
            python_numeric_value_key(&Constant::Complex {
                real: 1.0,
                imag: f64::NAN
            })
            .is_none()
        );
    }

    #[test]
    fn large_integer_mapping_keys_compare_exact_values() {
        let equal = [
            (
                "18446744073709551616",
                "18446744073709551616.0",
                "1.8446744073709552e+19",
            ),
            (
                "18446744073709551616.0",
                "18446744073709551616",
                "18446744073709551616",
            ),
            (
                "-18446744073709551616",
                "-18446744073709551616.0",
                "-1.8446744073709552e+19",
            ),
            (
                "-18446744073709551616.0",
                "-18446744073709551616",
                "-18446744073709551616",
            ),
            (
                "0x1_0000_0000_0000_0000",
                "18446744073709551616",
                "18446744073709551616",
            ),
            (
                "18446744073709551616",
                "18446744073709551617.0",
                "1.8446744073709552e+19",
            ),
            (
                "18446744073709551616",
                "18446744073709551616+0j",
                "(1.8446744073709552e+19+0j)",
            ),
            (
                "18446744073709551616+0j",
                "18446744073709551616",
                "18446744073709551616",
            ),
            (
                "18446744073709551616.0",
                "18446744073709551616+0j",
                "(1.8446744073709552e+19+0j)",
            ),
            (
                "18446744073709551616+0j",
                "18446744073709551616.0",
                "1.8446744073709552e+19",
            ),
            (
                "18446744073709551616+0j",
                "18446744073709551617+0j",
                "(1.8446744073709552e+19+0j)",
            ),
            (
                "18446744073709551617+0j",
                "18446744073709551616+0j",
                "(1.8446744073709552e+19+0j)",
            ),
            (
                "18446744073709551616+1j",
                "18446744073709551617+1j",
                "(1.8446744073709552e+19+1j)",
            ),
            (
                "18446744073709551617+1.5j",
                "18446744073709551616+1.5j",
                "(1.8446744073709552e+19+1.5j)",
            ),
            (
                "18446744073709551616+1e309j",
                "18446744073709551617+1e309j",
                "(1.8446744073709552e+19+infj)",
            ),
        ];
        for (first, second, repr) in equal {
            let mapping = format!("{{{first}: _, {second}: _}}");
            let source = format!("match subject:\n case [{mapping}]: pass\n");
            let parsed = parse_module(&source).unwrap();
            for target in TARGETS {
                let error = validate(&source, &parsed.syntax().body, target).unwrap_err();
                let CompileError::InvalidPattern { message, range } = error else {
                    panic!("{target}: {source:?}: {error:?}");
                };
                assert_eq!(
                    message,
                    format!("mapping pattern checks duplicate key ({repr})"),
                    "{target}: {source:?}"
                );
                assert_eq!(&source[range], mapping, "{target}: {source:?}");
            }
        }

        let too_large = format!("0x1{}", "0".repeat(256));
        let max_power = format!("0x8{}", "0".repeat(255));
        let unequal = [
            ("18446744073709551617", "18446744073709551616.0"),
            ("18446744073709551616", "18446744073709551616+1j"),
            ("18446744073709551616+1j", "18446744073709551617+2j"),
            ("18446744073709551617", "18446744073709551617+0j"),
            ("18446744073709551617+0j", "18446744073709551617"),
            ("18446744073709551616", "1e309"),
            ("9007199254740993", "9007199254740992.0"),
            (&too_large, "1e309"),
        ];
        for (first, second) in unequal {
            let source = format!("match subject:\n case {{{first}: _, {second}: _}}: pass\n");
            let parsed = parse_module(&source).unwrap();
            for target in TARGETS {
                validate(&source, &parsed.syntax().body, target)
                    .unwrap_or_else(|error| panic!("{target}: {source:?}: {error:?}"));
            }
        }
        let source =
            format!("match subject:\n case {{{max_power}: _, 8.98846567431158e307: _}}: pass\n");
        let parsed = parse_module(&source).unwrap();
        for target in TARGETS {
            assert!(
                matches!(
                    validate(&source, &parsed.syntax().body, target),
                    Err(CompileError::InvalidPattern { .. })
                ),
                "{target}: {source:?}"
            );
        }
    }

    #[test]
    fn large_duplicate_keys_keep_pattern_and_phase_error_order() {
        let big = "18446744073709551616";
        let float = "18446744073709551616.0";
        for (source, expected, span) in [
            (
                format!("match x:\n case {{{big}: [*a, *b], {float}: _}}: pass\n"),
                "mapping pattern checks duplicate key",
                format!("{{{big}: [*a, *b], {float}: _}}"),
            ),
            (
                format!("match x:\n case {{1: _, 1.0: _, {big}: _, {float}: _}}: pass\n"),
                "mapping pattern checks duplicate key (1.0)",
                format!("{{1: _, 1.0: _, {big}: _, {float}: _}}"),
            ),
            (
                format!("match x:\n case {{{big}: _, {float}: _, 1: _, 1.0: _}}: pass\n"),
                "mapping pattern checks duplicate key (1.8446744073709552e+19)",
                format!("{{{big}: _, {float}: _, 1: _, 1.0: _}}"),
            ),
            (
                format!("match x:\n case _: pass\n case {{{big}: _, {float}: _}}: pass\n"),
                "wildcard makes remaining patterns unreachable",
                "_".to_owned(),
            ),
            (
                format!("match x:\n case 1: break\n case {{{big}: _, {float}: _}}: pass\n"),
                "'break' outside loop",
                "break".to_owned(),
            ),
            (
                format!("match x:\n case {{{big}: _, {float}: _}}: pass\ny = 1\nglobal y\n"),
                "name 'y' is assigned to before global declaration",
                "global y".to_owned(),
            ),
        ] {
            let parsed = parse_module(&source).unwrap();
            for target in TARGETS {
                let error = validate(&source, &parsed.syntax().body, target).unwrap_err();
                let (message, range) = match error {
                    CompileError::InvalidPattern { message, range }
                    | CompileError::InvalidSyntax { message, range } => (message, range),
                    other => panic!("{target}: {source:?}: {other:?}"),
                };
                assert!(
                    message.contains(expected),
                    "{target}: {source:?}: {message:?}"
                );
                assert_eq!(&source[range], span, "{target}: {source:?}");
            }
        }

        let source = format!(
            "match x:\n case {{{big}: _, {float}: _}}: pass\nfrom __future__ import annotations\n"
        );
        let parsed = parse_module(&source).unwrap();
        for target in TARGETS {
            let error = validate(&source, &parsed.syntax().body, target).unwrap_err();
            let message = error.to_string();
            if target >= PythonVersion::Py314 {
                assert!(
                    message.contains("from __future__ imports must occur"),
                    "{target}: {message}"
                );
            } else {
                assert!(
                    message.contains("mapping pattern checks duplicate key"),
                    "{target}: {message}"
                );
            }
        }

        let mapping = format!("match v:\n case {{{big}: _, {float}: _}}: pass\n");
        for (source, legacy, modern) in [
            (
                format!("{mapping}await x\nfrom __future__ import annotations\n"),
                "mapping pattern checks duplicate key",
                "'await' outside function",
            ),
            (
                format!("{mapping}from __future__ import annotations\nawait x\n"),
                "mapping pattern checks duplicate key",
                "from __future__ imports must occur",
            ),
            (
                format!("x=1\nglobal x\n{mapping}await x\n"),
                "name 'x' is assigned to before global declaration",
                "name 'x' is assigned to before global declaration",
            ),
            (
                format!("{mapping}await x\nx=1\nglobal x\n"),
                "name 'x' is used prior to global declaration",
                "'await' outside function",
            ),
            (
                format!("def f():\n nonlocal missing\n{mapping}await x\n"),
                "no binding for nonlocal 'missing' found",
                "'await' outside function",
            ),
            (
                format!("x=1\nglobal x\n{mapping}from __future__ import annotations\n"),
                "name 'x' is assigned to before global declaration",
                "name 'x' is assigned to before global declaration",
            ),
            (
                format!("{mapping}from __future__ import annotations\nx=1\nglobal x\n"),
                "name 'x' is assigned to before global declaration",
                "from __future__ imports must occur",
            ),
            (
                format!("{mapping}x=1\nglobal x\nfrom __future__ import unknown\n"),
                "name 'x' is assigned to before global declaration",
                "name 'x' is assigned to before global declaration",
            ),
            (
                format!("{mapping}from __future__ import unknown\nx=1\nglobal x\n"),
                "name 'x' is assigned to before global declaration",
                "from __future__ imports must occur",
            ),
            (
                format!("{mapping}await x\nfrom __future__ import unknown\n"),
                "mapping pattern checks duplicate key",
                "'await' outside function",
            ),
            (
                format!("{mapping}from __future__ import unknown\n"),
                "mapping pattern checks duplicate key",
                "from __future__ imports must occur",
            ),
        ] {
            let parsed = parse_module(&source).unwrap();
            for target in TARGETS {
                let error = validate(&source, &parsed.syntax().body, target).unwrap_err();
                let message = error.to_string();
                let expected = if target >= PythonVersion::Py314 {
                    modern
                } else {
                    legacy
                };
                assert!(
                    message.contains(expected),
                    "{target}: {source:?}: {message}"
                );
            }
        }
    }

    #[test]
    fn large_duplicate_in_class_body_preserves_symbol_order() {
        let mapping = "{18446744073709551616: _, 18446744073709551616.0: _}";
        for (suffix, modern_message, modern_span) in [
            (
                "await x\nfrom __future__ import annotations\n",
                "'await' outside function",
                "await x",
            ),
            (
                "from __future__ import annotations\nawait x\n",
                "from __future__ imports must occur",
                "from __future__ import annotations",
            ),
        ] {
            let source = format!("class C(x=1, x=2):\n match v:\n  case {mapping}: pass\n{suffix}");
            let parsed = parse_module(&source).unwrap();
            for target in TARGETS {
                let error = validate(&source, &parsed.syntax().body, target).unwrap_err();
                let range = error.range().expect("diagnostic has a source range");
                let (message, span) = if target >= PythonVersion::Py314 {
                    (modern_message, modern_span)
                } else {
                    ("mapping pattern checks duplicate key", mapping)
                };
                assert!(error.to_string().contains(message), "{target}: {error:?}");
                assert_eq!(&source[range], span, "{target}: {error:?}");
            }
        }
    }
}
