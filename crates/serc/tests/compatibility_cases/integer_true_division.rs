use crate::common;

fn assert_ratio(numerator: &str, denominator: &str) {
    for (left_sign, right_sign) in [("", ""), ("-", ""), ("", "-"), ("-", "-")] {
        let source = format!(
            "def ratio():\n    return {left_sign}{numerator} / {right_sign}{denominator}\n"
        );
        common::assert_matches_cpython_marshal(&source, "integer_true_division.py");
    }
}

fn shifted_hex(value: u64, shift: usize) -> String {
    format!(
        "0x{:x}{}",
        u128::from(value) << (shift % 4),
        "0".repeat(shift / 4)
    )
}

#[test]
fn matches_cpython_integer_true_division_exact_rounding() {
    for (numerator, denominator) in [
        ("0", "3"),
        ("1", "2731"),
        ("9007199254740993", "3"),
        ("9007199254740995", "9007199254740991"),
        ("9007199254740993", "9007199254740992"),
        ("9007199254740995", "9007199254740992"),
        ("18446744073709551615", "18446744073709551616"),
        ("1", "18446744073709551616"),
        ("295147931372582273023", "295147932265116303360"),
    ] {
        assert_ratio(numerator, denominator);
    }
    assert_ratio(&shifted_hex(7, 4096), &shifted_hex(3, 4096));
    common::assert_matches_cpython_marshal(
        "values = (True / True, False / -3, 3 / True, (2**64 - 1) / 2**64, 1 / 2**64)\n",
        "integer_true_division_constants.py",
    );
}

#[test]
fn matches_cpython_integer_true_division_subnormal_rounding() {
    let denominator = shifted_hex(1, 1075);
    for numerator in [1_u64, 2, 3, 5, (1 << 53) - 1, 1 << 53, (1 << 53) + 1] {
        assert_ratio(&numerator.to_string(), &denominator);
    }
    assert_ratio("1", &shifted_hex(1, 8192));
}

#[test]
fn matches_cpython_integer_true_division_failure_boundaries() {
    for numerator in [(1 << 54) - 2, (1 << 54) - 1, 1 << 54] {
        assert_ratio(&shifted_hex(numerator, 970), "1");
    }
    assert_ratio(&shifted_hex(1, 8192), "1");
    assert_ratio("0", "0");
    assert_ratio("123456789012345678901234567890", "0");
    common::assert_matches_cpython_marshal(
        "def ratio():\n    try:\n        return 1 / False\n    except ZeroDivisionError:\n        return 0\n",
        "integer_true_division_failure.py",
    );
}
