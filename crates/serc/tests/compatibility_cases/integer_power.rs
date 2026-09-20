use crate::common;

#[test]
fn matches_cpython_integer_power_representations_and_limits() {
    for expression in [
        "(-2) ** 2",
        "(-2) ** 3",
        "2 ** True",
        "True ** 2",
        "False ** False",
        "False ** True",
        "(-2) ** True",
        "(-2) ** False",
        "True ** 128",
        "True ** 129",
        "(-2) ** 63",
        "(-2) ** 64",
        "(-2) ** 65",
        "(-3) ** 64",
        "0 ** 4294967296",
        "False ** 18446744073709551616",
        "0 ** 18446744073709551616",
        "18446744073709551616 ** 0",
        "18446744073709551616 ** 1",
        "18446744073709551616 ** 2",
        "(-18446744073709551616) ** 1",
        "(-18446744073709551616) ** 2",
        "18446744073709551616 ** False",
        "18446744073709551616 ** True",
        "(-2) ** -3",
        "(-2) ** -1075",
        "(-18446744073709551616) ** -17",
        "(-1) ** -9007199254740993",
        "True ** -1",
        "False ** -1",
        "0 ** -1",
        "18446744073709551616 ** -1",
        "(-1) ** -18446744073709551617",
        "2 ** -18446744073709551616",
        "2 ** 64",
        "2 ** 65",
    ] {
        common::assert_matches_cpython_marshal(
            &format!("def power():\n    return {expression}\n"),
            "integer_power.py",
        );
    }
    let huge = format!("0x1{}", "0".repeat(300));
    for expression in [
        format!("{huge} ** 0"),
        format!("{huge} ** 1"),
        format!("{huge} ** -1"),
        format!("0 ** {huge}"),
        format!("1 ** -{huge}"),
    ] {
        common::assert_matches_cpython_marshal(
            &format!("def power():\n    return {expression}\n"),
            "integer_power_huge.py",
        );
    }
}
