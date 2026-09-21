use crate::common;

#[test]
fn matches_cpython_negative_real_fractional_power_folding() {
    for base in ["-1", "-2.0", "-18446744073709553665", "-1e-100", "-1e100"] {
        for exponent in ["0.2", "0.5", "1.5", "-0.2", "-1.5"] {
            let source = format!("def power():\n    return ({base}) ** ({exponent})\n");
            common::assert_matches_cpython_marshal(&source, "negative_fractional_power.py");
        }
    }
}

#[test]
fn matches_cpython_fractional_power_limits_and_real_controls() {
    for expression in [
        "(-1e308) ** 1.5",
        "(-1e-308) ** -1.5",
        "(-1e-308) ** 1.5",
        "(-1e308) ** -1.5",
        "(-5e-324) ** 0.5",
        "(-5e-324) ** 1.5",
        "(-2.0) ** 0.0",
        "(-2.0) ** 3.0",
        "(-2.0) ** -3.0",
        "(-0.0) ** 0.5",
        "(-0.0) ** -0.5",
        "2.0 ** 0.5",
        "2 ** -0.5",
        "True ** 0.5",
        "(-1e309) ** -0.5",
        "(-2.0) ** (-1e309)",
    ] {
        let source = format!("def power():\n    return {expression}\n");
        common::assert_matches_cpython_marshal(&source, "fractional_power_limits.py");
    }
}
