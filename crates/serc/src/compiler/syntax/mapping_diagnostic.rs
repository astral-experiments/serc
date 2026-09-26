//! CPython's diagnostic renders the repeated value, rather than its source spelling.

use std::fmt::Write;

use ruff_python_ast::visitor::{Visitor, walk_pattern, walk_stmt};
use ruff_python_ast::{self as ast, Expr, Pattern, Stmt};
use ruff_python_literal::escape::AsciiEscape;
use ruff_text_size::{Ranged, TextRange};

use crate::{PythonVersion, code_object::Constant};

use super::super::constant::{fold_constant_with_literals, literal_constant};
use super::super::source::diagnostic_string_literal_repr;
use super::unicode_printable;

/// Resolve Ruff's key location to its enclosing mapping and native diagnostic.
pub(super) fn find(
    source: &str,
    body: &[Stmt],
    key_range: TextRange,
    version: ast::PythonVersion,
) -> Option<(String, TextRange)> {
    struct Finder<'source> {
        source: &'source str,
        key_range: TextRange,
        version: ast::PythonVersion,
        found: Option<(String, TextRange)>,
    }

    impl<'ast> Visitor<'ast> for Finder<'_> {
        fn visit_stmt(&mut self, statement: &'ast Stmt) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_stmt(statement));
            }
            if self.found.is_none() && statement.range().contains_range(self.key_range) {
                walk_stmt(self, statement);
            }
        }

        fn visit_pattern(&mut self, pattern: &'ast Pattern) {
            if crate::stack_growth_required() {
                return crate::grow_stack(|| self.visit_pattern(pattern));
            }
            if self.found.is_some() || !pattern.range().contains_range(self.key_range) {
                return;
            }
            if let Pattern::MatchMapping(mapping) = pattern
                && let Some(key) = mapping
                    .keys
                    .iter()
                    .find(|key| key.range() == self.key_range)
            {
                self.found = diagnostic(self.source, key, mapping.range, self.version);
            } else {
                walk_pattern(self, pattern);
            }
        }

        fn visit_expr(&mut self, _expression: &'ast Expr) {}
    }

    let mut finder = Finder {
        source,
        key_range,
        version,
        found: None,
    };
    finder.visit_body(body);
    finder.found
}

/// Format the repeated constant while retaining the entire mapping as the error span.
pub(super) fn diagnostic(
    source: &str,
    key: &Expr,
    mapping_range: TextRange,
    version: ast::PythonVersion,
) -> Option<(String, TextRange)> {
    let repr = match key {
        Expr::StringLiteral(string) => {
            diagnostic_string_literal_repr(string, source, |character| {
                unicode_printable::is_printable(character, version)
            })
        }
        _ => {
            // Avoid unbounded big-integer conversion before applying the decimal-size guard.
            if matches!(
                key,
                Expr::NumberLiteral(_) | Expr::UnaryOp(_) | Expr::BinOp(_)
            ) && source[key.range()].len() > 8192
            {
                return None;
            }
            let version = match version {
                ast::PythonVersion::PY312 => PythonVersion::Py312,
                ast::PythonVersion::PY313 => PythonVersion::Py313,
                ast::PythonVersion::PY314 => PythonVersion::Py314,
                _ => PythonVersion::Py315,
            };
            constant_repr(&fold_constant_with_literals(
                key,
                version,
                &literal_constant,
                false,
            )?)?
        }
    };
    Some((
        format!("mapping pattern checks duplicate key ({repr})"),
        mapping_range,
    ))
}

fn constant_repr(constant: &Constant) -> Option<String> {
    match constant {
        Constant::None => Some("None".to_owned()),
        Constant::Bool(value) => Some(if *value { "True" } else { "False" }.to_owned()),
        Constant::Int(value) => Some(value.to_string()),
        Constant::SignedInt(value) => Some(value.to_string()),
        // CPython's default integer-to-string limit rejects values above 4,300 decimal digits.
        // Keep the existing diagnostic for larger values rather than doing unbounded quadratic
        // conversion while Serc has no equivalent configurable limit.
        Constant::BigInt { negative, digits } if digits.len() <= 954 => {
            Some(integer_repr(*negative, digits))
        }
        Constant::Float(value) => Some(float_repr(*value)),
        Constant::Complex { real, imag } => Some(complex_repr(*real, *imag)),
        Constant::Bytes(value) => Some(
            AsciiEscape::new_repr(value)
                .bytes_repr(ruff_python_ast::str::TripleQuotes::No)
                .to_string()
                .expect("Python bytes representation length fits in usize"),
        ),
        _ => None,
    }
}

fn integer_repr(negative: bool, digits: &[u16]) -> String {
    // Convert marshal's base-2^15 limbs to decimal chunks without narrowing the value.
    let mut chunks = vec![0_u32];
    for digit in digits.iter().rev() {
        let mut carry = u64::from(*digit);
        for chunk in &mut chunks {
            let value = u64::from(*chunk) * (1 << 15) + carry;
            *chunk = u32::try_from(value % 1_000_000_000).expect("decimal chunk fits in u32");
            carry = value / 1_000_000_000;
        }
        if carry != 0 {
            chunks.push(u32::try_from(carry).expect("decimal carry fits in u32"));
        }
    }
    let mut result = if negative {
        "-".to_owned()
    } else {
        String::new()
    };
    let mut chunks = chunks.iter().rev();
    write!(result, "{}", chunks.next().expect("at least one chunk")).unwrap();
    for chunk in chunks {
        write!(result, "{chunk:09}").unwrap();
    }
    result
}

fn float_repr(value: f64) -> String {
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf"
        } else {
            "inf"
        }
        .to_owned();
    }
    if value == 0.0 {
        return if value.is_sign_negative() {
            "-0.0"
        } else {
            "0.0"
        }
        .to_owned();
    }

    let negative = value.is_sign_negative();
    let raw = value.abs().to_string();
    let dot = raw.find('.').unwrap_or(raw.len());
    let digits: String = raw.chars().filter(|&character| character != '.').collect();
    let first = digits
        .find(|character| character != '0')
        .expect("nonzero value");
    let last = digits
        .rfind(|character| character != '0')
        .expect("nonzero value");
    let exponent = i32::try_from(dot).unwrap() - i32::try_from(first).unwrap() - 1;
    let significant = &digits[first..=last];

    // Rust rounds exact decimal midpoints away from zero; Python chooses the even digit.
    // Check the midpoint as factored integers, avoiding floating-point rounding here.
    let number: u64 = significant
        .parse()
        .expect("shortest float has at most 17 digits");
    let scale = exponent - i32::try_from(significant.len()).unwrap() + 1;
    let bits = value.abs().to_bits();
    let exponent_bits = i32::try_from((bits >> 52) & 0x7ff).unwrap();
    let fraction = bits & ((1 << 52) - 1);
    let mantissa = if exponent_bits == 0 {
        fraction
    } else {
        fraction | (1 << 52)
    };
    let binary_exp = if exponent_bits == 0 {
        -1074
    } else {
        exponent_bits - 1023 - 52
    };
    let trailing = i32::try_from(mantissa.trailing_zeros()).unwrap();
    let odd = mantissa >> trailing;
    let midpoint = 2 * u128::from(number) - 1;
    let tie = if (0..=23).contains(&scale) {
        u128::from(odd) == midpoint * 5_u128.pow(u32::try_from(scale).unwrap())
            && binary_exp + 1 + trailing == scale
    } else if (-25..0).contains(&scale) {
        let power = -scale;
        u128::from(odd) * 5_u128.pow(u32::try_from(power).unwrap()) == midpoint
            && binary_exp + 1 + trailing + power == 0
    } else {
        false
    };
    let adjusted = (number % 2 == 1 && tie).then(|| (number - 1).to_string());
    let significant = adjusted
        .as_deref()
        .unwrap_or(significant)
        .trim_end_matches('0');
    let sign = if negative { "-" } else { "" };

    if !(-4..16).contains(&exponent) {
        let mut result = format!("{sign}{}", &significant[..1]);
        if significant.len() > 1 {
            result.push('.');
            result.push_str(&significant[1..]);
        }
        write!(result, "e{exponent:+03}").unwrap();
        result
    } else if exponent < 0 {
        format!(
            "{sign}0.{}{significant}",
            "0".repeat(usize::try_from(-exponent - 1).unwrap())
        )
    } else {
        let before = usize::try_from(exponent).unwrap() + 1;
        if significant.len() <= before {
            format!(
                "{sign}{significant}{}.0",
                "0".repeat(before - significant.len())
            )
        } else {
            format!(
                "{sign}{}.{}",
                &significant[..before],
                &significant[before..]
            )
        }
    }
}

fn complex_repr(real: f64, imag: f64) -> String {
    fn component(value: f64) -> String {
        let repr = float_repr(value);
        repr.strip_suffix(".0").unwrap_or(&repr).to_owned()
    }
    if real == 0.0 && !real.is_sign_negative() {
        format!("{}j", component(imag))
    } else {
        let sign = if imag.is_sign_negative() { '-' } else { '+' };
        format!("({}{sign}{}j)", component(real), component(imag.abs()))
    }
}

#[cfg(test)]
mod tests {
    use ruff_python_parser::parse_module;

    use super::super::validate;
    use crate::{CompileError, PythonVersion};

    #[test]
    fn duplicate_mapping_keys_use_native_repr_and_mapping_span() {
        let cases = [
            ("{1: a, True: b}", "True"),
            ("{True: a, 0x1: b}", "1"),
            ("{1: a, 1_0e-1: b}", "1.0"),
            ("{1: a, 1+0j: b}", "(1+0j)"),
            ("{0: a, -0.0: b}", "-0.0"),
            ("{0: a, -0j: b}", "(-0-0j)"),
            ("{1e-5: a, 1e-5: b}", "1e-05"),
            ("{1e16: a, 1e16: b}", "1e+16"),
            ("{1e15: a, 1e15: b}", "1000000000000000.0"),
            (
                "{206663914597396.625: a, 206663914597396.625: b}",
                "206663914597396.62",
            ),
            (
                "{340282366920938463463374607431768211455: a, 340282366920938463463374607431768211455: b}",
                "340282366920938463463374607431768211455",
            ),
            ("{'\\x61': a, \"a\": b}", "'a'"),
            ("{'\\ud800': a, \"\\ud800\": b}", "'\\ud800'"),
            ("{'a' 'b': a, \"ab\": b}", "'ab'"),
            ("{\"a' b\": a, 'a\\\' b': b}", "\"a' b\""),
            ("{b'\\xff': a, b\"\\xff\": b}", "b'\\xff'"),
            ("{None: a, None: b}", "None"),
        ];
        for (mapping, repr) in cases {
            let source = format!("def f(subject):\n match subject:\n  case [{mapping}]: pass\n");
            let parsed = parse_module(&source).unwrap();
            for version in [
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ] {
                let error = validate(&source, &parsed.syntax().body, version).unwrap_err();
                let CompileError::InvalidPattern { message, range } = error else {
                    panic!("unexpected diagnostic for {version}, {source:?}: {error:?}");
                };
                assert_eq!(
                    message,
                    format!("mapping pattern checks duplicate key ({repr})"),
                    "{version}: {source:?}"
                );
                assert_eq!(&source[range], mapping, "{version}: {source:?}");
            }
        }
    }

    #[test]
    fn complex_key_repr_uses_the_target_folding_rules() {
        for (mapping, legacy, modern) in [
            ("{0: a, 0-0j: b}", "0j", "-0j"),
            ("{1: a, 1-0j: b}", "(1+0j)", "(1-0j)"),
            ("{0: a, -0.0-0j: b}", "(-0+0j)", "(-0-0j)"),
            ("{1e999: a, 1e999-0j: b}", "(inf+0j)", "(inf-0j)"),
        ] {
            let source = format!("match subject:\n case {mapping}: pass\n");
            let parsed = parse_module(&source).unwrap();
            for version in [
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ] {
                let expected = if version <= PythonVersion::Py313 {
                    legacy
                } else {
                    modern
                };
                let error = validate(&source, &parsed.syntax().body, version).unwrap_err();
                let CompileError::InvalidPattern { message, range } = error else {
                    panic!("unexpected diagnostic for {version}, {source:?}: {error:?}");
                };
                assert_eq!(
                    message,
                    format!("mapping pattern checks duplicate key ({expected})"),
                    "{version}: {source:?}"
                );
                assert_eq!(&source[range], mapping, "{version}: {source:?}");
            }
        }
    }

    #[test]
    fn string_key_repr_uses_the_target_unicode_version() {
        for (mapping, expectations) in [
            (
                "{'\\u1c89': a, '\\u1c89': b}",
                ["'\\u1c89'", "'\\u1c89'", "'\u{1c89}'", "'\u{1c89}'"],
            ),
            (
                "{'\\U0001faea': a, '\\U0001faea': b}",
                [
                    "'\\U0001faea'",
                    "'\\U0001faea'",
                    "'\\U0001faea'",
                    "'\u{1faea}'",
                ],
            ),
        ] {
            let source = format!("match subject:\n case {mapping}: pass\n");
            let parsed = parse_module(&source).unwrap();
            for (version, expected) in [
                PythonVersion::Py312,
                PythonVersion::Py313,
                PythonVersion::Py314,
                PythonVersion::Py315,
            ]
            .into_iter()
            .zip(expectations)
            {
                let error = validate(&source, &parsed.syntax().body, version).unwrap_err();
                let CompileError::InvalidPattern { message, range } = error else {
                    panic!("unexpected diagnostic for {version}, {source:?}: {error:?}");
                };
                assert_eq!(
                    message,
                    format!("mapping pattern checks duplicate key ({expected})"),
                    "{version}: {source:?}"
                );
                assert_eq!(&source[range], mapping, "{version}: {source:?}");
            }
        }
    }

    #[test]
    fn duplicate_key_diagnostics_preserve_legacy_debug_pattern_order() {
        for (mapping, repr) in [
            ("{1: _, True: C(__debug__=x)}", "True"),
            (
                "{18446744073709551616: _, 18446744073709551616.0: C(__debug__=x)}",
                "1.8446744073709552e+19",
            ),
        ] {
            let source = format!("match subject:\n case {mapping}: pass\n");
            let parsed = parse_module(&source).unwrap();
            for version in [PythonVersion::Py312, PythonVersion::Py313] {
                let error = validate(&source, &parsed.syntax().body, version).unwrap_err();
                let CompileError::InvalidPattern { message, range } = error else {
                    panic!("unexpected diagnostic for {version}, {source:?}: {error:?}");
                };
                assert_eq!(
                    message,
                    format!("mapping pattern checks duplicate key ({repr})"),
                    "{version}: {source:?}"
                );
                assert_eq!(&source[range], mapping, "{version}: {source:?}");
            }
        }
    }
}
