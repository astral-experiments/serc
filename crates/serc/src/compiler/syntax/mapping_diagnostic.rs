//! CPython's diagnostic renders the repeated value, rather than its source spelling.

use std::fmt::Write;

use ruff_python_ast::visitor::{Visitor, walk_pattern, walk_stmt};
use ruff_python_ast::{self as ast, Expr, Pattern, Stmt};
use ruff_python_literal::escape::AsciiEscape;
use ruff_text_size::{Ranged, TextRange};

use crate::{CompileError, PythonVersion, code_object::Constant};

use super::super::constant::{fold_constant_with_literals, literal_constant};
use super::super::source::diagnostic_string_literal_repr;
use super::unicode_printable;

/// Resolve Ruff's key location to its enclosing mapping and native diagnostic.
pub(super) fn find(
    source: &str,
    body: &[Stmt],
    key_range: TextRange,
    version: ast::PythonVersion,
    int_max_str_digits: Option<u32>,
) -> Option<CompileError> {
    struct Finder<'source> {
        source: &'source str,
        key_range: TextRange,
        version: ast::PythonVersion,
        int_max_str_digits: Option<u32>,
        found: Option<CompileError>,
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
                self.found = diagnostic(
                    self.source,
                    key,
                    mapping.range,
                    self.version,
                    self.int_max_str_digits,
                );
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
        int_max_str_digits,
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
    int_max_str_digits: Option<u32>,
) -> Option<CompileError> {
    let repr = if let Expr::StringLiteral(string) = key {
        diagnostic_string_literal_repr(string, source, |character| {
            unicode_printable::is_printable(character, version)
        })
    } else {
        // Avoid unbounded big-integer conversion. Do not claim a syntax error when the
        // unconverted integer could instead exceed the caller's conversion limit.
        if matches!(
            key,
            Expr::NumberLiteral(_) | Expr::UnaryOp(_) | Expr::BinOp(_)
        ) && source[key.range()].len() > 8192
        {
            if is_integer_literal(key) {
                return Some(CompileError::Unsupported(
                    "integer mapping-key diagnostic exceeds the supported conversion size"
                        .to_owned(),
                ));
            }
            return None;
        }
        let version = match version {
            ast::PythonVersion::PY312 => PythonVersion::Py312,
            ast::PythonVersion::PY313 => PythonVersion::Py313,
            ast::PythonVersion::PY314 => PythonVersion::Py314,
            _ => PythonVersion::Py315,
        };
        match constant_repr(
            &fold_constant_with_literals(key, version, &literal_constant, false)?,
            int_max_str_digits,
        ) {
            Ok(repr) => repr?,
            Err(error) => return Some(error),
        }
    };
    Some(CompileError::InvalidPattern {
        message: format!("mapping pattern checks duplicate key ({repr})"),
        range: mapping_range,
    })
}

fn is_integer_literal(key: &Expr) -> bool {
    match key {
        Expr::NumberLiteral(number) => matches!(&number.value, ast::Number::Int(_)),
        Expr::UnaryOp(unary) if matches!(unary.op, ast::UnaryOp::UAdd | ast::UnaryOp::USub) => {
            is_integer_literal(&unary.operand)
        }
        _ => false,
    }
}

fn constant_repr(
    constant: &Constant,
    int_max_str_digits: Option<u32>,
) -> Result<Option<String>, CompileError> {
    Ok(match constant {
        Constant::None => Some("None".to_owned()),
        Constant::Bool(value) => Some(if *value { "True" } else { "False" }.to_owned()),
        Constant::Int(value) => Some(value.to_string()),
        Constant::SignedInt(value) => Some(value.to_string()),
        Constant::BigInt { negative, digits } => {
            // CPython supports 15- and 30-bit PyLong digits, while these normalized
            // marshal digits are 15-bit. Above 1,000 native digits CPython can invoke
            // the caller-controlled _pylong module before its exact limit check.
            // Claim ValueError only if the coarse precheck rejects in both layouts.
            if digits.len() > 1_000 {
                let native_digits_15 = digits.len();
                let native_digits_30 = digits.len().div_ceil(2);
                if let Some(limit @ 1..) = int_max_str_digits
                    && (limit as usize) / 45 <= (native_digits_15 - 11) / 10
                    && (limit as usize) / 90 <= (native_digits_30 - 11) / 10
                {
                    return Err(CompileError::IntegerStringConversionLimit { max_digits: limit });
                }
                return Err(CompileError::Unsupported(
                    "integer formatting may invoke the caller-controlled _pylong module".to_owned(),
                ));
            }
            let repr = integer_repr(*negative, digits);
            let decimal_digits = repr.len() - usize::from(*negative);
            if decimal_digits > 640 {
                match int_max_str_digits {
                    None => return Err(CompileError::UnresolvedIntegerStringLimit),
                    Some(limit) if limit != 0 && decimal_digits > limit as usize => {
                        return Err(CompileError::IntegerStringConversionLimit {
                            max_digits: limit,
                        });
                    }
                    Some(_) => {}
                }
            }
            Some(repr)
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
    })
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
    use crate::{
        CompileError, CompileOptions, OptimizationLevel, PythonVersion, compile_with_options,
        compile_with_options_and_mapping_key_limit,
    };

    fn power_of_two(exponent: usize) -> String {
        format!("0x{:x}{}", 1_u8 << (exponent % 4), "0".repeat(exponent / 4))
    }

    fn duplicate_integer_error(
        key: &str,
        limit: Option<u32>,
        version: PythonVersion,
    ) -> CompileError {
        let source = format!("match subject:\n case {{{key}: _, {key}: _}}: pass\n");
        compile_error(&source, version, limit)
    }

    fn compile_error(source: &str, version: PythonVersion, limit: Option<u32>) -> CompileError {
        let options = CompileOptions {
            python_version: version,
            ..CompileOptions::default()
        };
        match limit {
            Some(limit) => {
                compile_with_options_and_mapping_key_limit(source, "mapping.py", &options, limit)
            }
            None => compile_with_options(source, "mapping.py", &options),
        }
        .unwrap_err()
    }

    #[test]
    fn integer_repr_limit_is_preserved_with_optimization() {
        let key = power_of_two(2127);
        let source = format!("match subject:\n case {{{key}: _, {key}: _}}: pass\n");
        for python_version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for optimization_level in [OptimizationLevel::One, OptimizationLevel::Two] {
                let options = CompileOptions {
                    python_version,
                    optimization_level,
                    ..CompileOptions::default()
                };
                assert_eq!(
                    compile_with_options_and_mapping_key_limit(
                        &source,
                        "mapping.py",
                        &options,
                        640
                    )
                    .unwrap_err(),
                    CompileError::IntegerStringConversionLimit { max_digits: 640 },
                );
            }
        }
    }

    #[test]
    fn integer_repr_limit_is_unknown_unless_established_by_the_caller() {
        let below = power_of_two(2126); // 640 decimal digits.
        let above = power_of_two(2127); // 641 decimal digits.
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            assert!(matches!(
                duplicate_integer_error(&below, None, version),
                CompileError::InvalidPattern { .. }
            ));
            for sign in ["", "-"] {
                let key = format!("{sign}{above}");
                assert!(matches!(
                    duplicate_integer_error(&key, None, version),
                    CompileError::UnresolvedIntegerStringLimit
                ));
                assert_eq!(
                    duplicate_integer_error(&key, Some(640), version),
                    CompileError::IntegerStringConversionLimit { max_digits: 640 }
                );
                for limit in [0, 641, 4300] {
                    let error = duplicate_integer_error(&key, Some(limit), version);
                    let CompileError::InvalidPattern { message, .. } = error else {
                        panic!("expected duplicate-key diagnostic for {version}: {error:?}");
                    };
                    let repr = message
                        .strip_prefix("mapping pattern checks duplicate key (")
                        .unwrap()
                        .strip_suffix(')')
                        .unwrap();
                    assert_eq!(repr.trim_start_matches('-').len(), 641);
                    assert_eq!(repr.starts_with('-'), sign == "-");
                }
            }
        }
    }

    #[test]
    fn integer_repr_limit_checks_default_boundary_and_configuration() {
        let below = power_of_two(14284); // 4300 decimal digits.
        let above = power_of_two(14285); // 4301 decimal digits.
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            assert!(matches!(
                duplicate_integer_error(&below, Some(4300), version),
                CompileError::InvalidPattern { .. }
            ));
            assert_eq!(
                duplicate_integer_error(&above, Some(4300), version),
                CompileError::IntegerStringConversionLimit { max_digits: 4300 }
            );
            assert!(matches!(
                duplicate_integer_error(&above, Some(0), version),
                CompileError::InvalidPattern { .. }
            ));
            assert!(matches!(
                duplicate_integer_error(&above, Some(4301), version),
                CompileError::InvalidPattern { .. }
            ));
            assert!(matches!(
                duplicate_integer_error(&above, None, version),
                CompileError::UnresolvedIntegerStringLimit
            ));
        }
        for limit in [1, 639, u32::MAX] {
            assert!(matches!(
                duplicate_integer_error("1", Some(limit), PythonVersion::Py314),
                CompileError::Unsupported(_)
            ));
        }
    }

    #[test]
    fn integer_formatting_does_not_assume_the_parse_limit_or_pylong_module() {
        let key = power_of_two(2127);
        let source = format!(
            "match subject:\n case {{{key}: _, {key}: _}}: pass\nlater = {}\n",
            "1".repeat(641)
        );
        let options = CompileOptions::default();
        assert!(matches!(
            compile_with_options_and_mapping_key_limit(&source, "mapping.py", &options, 640),
            Err(CompileError::Unsupported(_))
        ));
        let source = format!(
            "# {}\nmessage = '{}'; match_subject = 1\nmatch subject:\n case {{{key}: _, {key}: _}}: pass\n",
            "1".repeat(641),
            "1".repeat(641)
        );
        assert!(matches!(
            compile_with_options_and_mapping_key_limit(&source, "mapping.py", &options, 640),
            Err(CompileError::IntegerStringConversionLimit { max_digits: 640 })
        ));

        let huge = power_of_two(30000); // 1,001 native 30-bit limbs.
        let ambiguous_layout = power_of_two(18000); // 1,201 15-bit limbs, 601 30-bit limbs.
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            assert_eq!(
                duplicate_integer_error(&huge, Some(4300), version),
                CompileError::IntegerStringConversionLimit { max_digits: 4300 }
            );
            assert!(matches!(
                duplicate_integer_error(&huge, Some(9000), version),
                CompileError::Unsupported(_)
            ));
            assert!(matches!(
                duplicate_integer_error(&huge, Some(0), version),
                CompileError::Unsupported(_)
            ));
            assert!(matches!(
                duplicate_integer_error(&ambiguous_layout, Some(5400), version),
                CompileError::Unsupported(_)
            ));
        }
    }

    #[test]
    fn duplicate_key_diagnostics_keep_earlier_phase_errors() {
        let key = power_of_two(2127);
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let options = CompileOptions {
                python_version: version,
                ..CompileOptions::default()
            };
            let source = format!(
                "match subject:\n case {{{key}: _, {key}: _}}: pass\ndef f():\n nonlocal missing\n"
            );
            let error =
                compile_with_options_and_mapping_key_limit(&source, "mapping.py", &options, 640)
                    .unwrap_err();
            assert!(
                matches!(error, CompileError::InvalidSyntax { .. }),
                "{version}: {error:?}"
            );
            assert!(
                error
                    .to_string()
                    .contains("no binding for nonlocal 'missing'"),
                "{version}: {error:?}"
            );
            let source = format!(
                "match subject:\n case {{1: _, 1: _}}: pass\nlater = {}\n",
                "1".repeat(641)
            );
            assert!(matches!(
                compile_with_options_and_mapping_key_limit(&source, "mapping.py", &options, 640),
                Err(CompileError::Unsupported(_))
            ));
        }
        for version in [PythonVersion::Py312, PythonVersion::Py313] {
            for key in ["1".to_owned(), key.clone()] {
                let source = format!(
                    "*items\nmatch subject:\n case {{{key}: _, {key}: _, **__debug__}}: pass\n"
                );
                let options = CompileOptions {
                    python_version: version,
                    ..CompileOptions::default()
                };
                let error = compile_with_options_and_mapping_key_limit(
                    &source,
                    "mapping.py",
                    &options,
                    640,
                )
                .unwrap_err();
                assert!(
                    error.to_string().contains("starred"),
                    "{version}: {error:?}"
                );
            }
        }
    }

    #[test]
    fn unknown_decimal_parse_precedes_earlier_semantic_and_parser_errors() {
        let long = "1".repeat(641);
        let separators = format!("{}{}", "1_".repeat(640), "1");
        let mapping = "match subject:\n case {1: _, 1: _}: pass\n";
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for prefix in [
                "",
                "def f():\n nonlocal missing\n",
                "if:\n pass\n",
                "open = (\n",
            ] {
                for decimal in [&long, &separators] {
                    let source = format!("{prefix}{mapping}later = {decimal}\n");
                    assert!(
                        matches!(
                            compile_error(&source, version, Some(640)),
                            CompileError::Unsupported(_)
                        ),
                        "{version}: {prefix:?}"
                    );
                }
            }
            let source = format!("# {long}\ns = '{long}'\n{mapping}");
            assert!(matches!(
                compile_error(&source, version, None),
                CompileError::InvalidPattern { .. }
            ));
        }
    }

    #[test]
    fn unknown_decimal_parse_guard_is_limited_to_possible_mapping_duplicates() {
        let decimal = "1".repeat(641);
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for pattern in ["_", "[item]", "{1: item}"] {
                let source = format!("match subject:\n case {pattern}: pass\nlater = {decimal}\n");
                let parsed = parse_module(&source).unwrap();
                assert!(
                    !super::super::may_have_duplicate_mapping_key(&parsed.syntax().body),
                    "{version}: {pattern}"
                );
            }
            for pattern in ["{1: _, 2: _}", "[{1: _, 1: _}]", "{1: _, 1: _} | {2: _}"] {
                let source = format!("match subject:\n case {pattern}: pass\nlater = {decimal}\n");
                assert!(
                    matches!(
                        compile_error(&source, version, Some(640)),
                        CompileError::Unsupported(_)
                    ),
                    "{version}: {pattern}"
                );
            }
        }
    }

    #[test]
    fn integer_limit_uses_existing_ordered_symbol_replay() {
        let key = power_of_two(2127);
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            let mapping = format!("match subject:\n case {{{key}: _, {key}: _}}: pass\n");
            for (suffix, expected_modern) in [
                (
                    "await subject\nfrom __future__ import annotations\n",
                    "'await' outside function",
                ),
                (
                    "from __future__ import annotations\nawait subject\n",
                    "from __future__ imports must occur",
                ),
            ] {
                let error = compile_error(&(mapping.clone() + suffix), version, Some(640));
                if matches!(version, PythonVersion::Py312 | PythonVersion::Py313) {
                    assert_eq!(
                        error,
                        CompileError::IntegerStringConversionLimit { max_digits: 640 }
                    );
                } else {
                    assert!(
                        matches!(error, CompileError::InvalidSyntax { .. })
                            && error.to_string().contains(expected_modern),
                        "{version}: {error:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn integer_limit_uses_debug_pattern_replay() {
        let key = power_of_two(2127);
        for version in [
            PythonVersion::Py312,
            PythonVersion::Py313,
            PythonVersion::Py314,
            PythonVersion::Py315,
        ] {
            for pattern in [
                format!("{{{key}: _, {key}: _, **__debug__}}"),
                format!("[{{{key}: _, {key}: _}}, C(__debug__=x)]"),
            ] {
                let source = format!("match subject:\n case {pattern}: pass\n");
                for limit in [None, Some(0), Some(640)] {
                    let error = compile_error(&source, version, limit);
                    if matches!(version, PythonVersion::Py314 | PythonVersion::Py315) {
                        assert!(
                            matches!(error, CompileError::InvalidSyntax { .. })
                                && error.to_string().contains("cannot assign to __debug__"),
                            "{version}: {error:?}"
                        );
                    } else {
                        match limit {
                            None => {
                                assert!(matches!(
                                    error,
                                    CompileError::UnresolvedIntegerStringLimit
                                ));
                            }
                            Some(0) => {
                                assert!(matches!(error, CompileError::InvalidPattern { .. }));
                            }
                            Some(640) => assert_eq!(
                                error,
                                CompileError::IntegerStringConversionLimit { max_digits: 640 }
                            ),
                            _ => unreachable!(),
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn explicit_limit_preserves_existing_options_literals_and_input_forms() {
        use crate::{
            InitialInternState, NativeBuildProfile,
            compile_bytes_with_options_and_mapping_key_limit,
            compile_bytes_with_path_options_and_mapping_key_limit,
        };
        use std::path::Path;
        let options = CompileOptions {
            python_version: PythonVersion::Py314,
            native_build_profile: NativeBuildProfile::default(),
            initial_intern_state: InitialInternState::default(),
            optimization_level: OptimizationLevel::Zero,
        };
        let key = power_of_two(2127);
        let source = format!("match subject:\n case {{{key}: _, {key}: _}}: pass\n");
        let expected = CompileError::IntegerStringConversionLimit { max_digits: 640 };
        assert_eq!(
            compile_with_options_and_mapping_key_limit(&source, "mapping.py", &options, 640)
                .unwrap_err(),
            expected
        );
        assert_eq!(
            compile_bytes_with_options_and_mapping_key_limit(
                source.as_bytes(),
                "mapping.py",
                &options,
                640
            )
            .unwrap_err(),
            expected
        );
        assert_eq!(
            compile_bytes_with_path_options_and_mapping_key_limit(
                source.as_bytes(),
                Path::new("mapping.py"),
                &options,
                640
            )
            .unwrap_err(),
            expected
        );
    }

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
