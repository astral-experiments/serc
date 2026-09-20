use crate::common;

#[test]
fn matches_cpython_marshal_for_integer_true_division_rounding() {
    for (name, expression) in [
        ("inexact_numerator", "9007199254740993 / 3"),
        ("inexact_denominator", "1 / 9007199254740993"),
        ("both_inexact", "9007199254740993 / 9007199254740995"),
        ("negative_numerator", "-9007199254740993 / 3"),
        ("signed_minimum", "-9223372036854775808 / 3"),
        ("positive_signed_numerator", "(~-2) / 3"),
        ("zero_signed_numerator", "(~-1) / 3"),
        ("minimum_exponent", "1 / 18446744073709551615"),
        ("maximum_exponent", "18446744073709551615 / 1"),
        (
            "equal_large_operands",
            "18446744073709551615 / 18446744073709551615",
        ),
        ("significand_boundary", "4503599627370496 / 1"),
        ("tie_to_even_lower", "9007199254740993 / 1"),
        ("tie_to_even_upper", "9007199254740995 / 1"),
        (
            "fraction_tie_to_even_lower",
            "9007199254740993 / 18014398509481984",
        ),
        (
            "fraction_tie_to_even_upper",
            "9007199254740995 / 18014398509481984",
        ),
        ("rounding_carry", "18014398509481983 / 18014398509481984"),
        ("exact_fraction", "1 / 2"),
        ("inexact_fraction", "1 / 3"),
        ("zero", "0 / 18446744073709551615"),
        ("negative_zero_integer", "-0 / 1"),
        ("division_by_zero", "1 / 0"),
        ("float_denominator_control", "9007199254740993 / 3.0"),
        ("float_numerator_control", "9007199254740993.0 / 3"),
    ] {
        common::assert_matches_cpython_marshal(
            &format!("value = {expression}\n"),
            &format!("integer_true_division_{name}.py"),
        );
    }
}
