use std::fmt::Write as _;

use crate::common;

#[test]
fn matches_cpython_nan_constant_identity() {
    let mut indexed_prefix = String::new();
    for value in 300..315 {
        writeln!(indexed_prefix, "value{value} = {value}").unwrap();
    }
    for expression in [
        "1e999 - 1e999",
        "1e999j - 1e999j",
        "0.0 * 1e999j",
        "-(1e999 - 1e999)",
        "(1e999 - 1e999) + 1j",
        "-(1e999j - 1e999j) * 2",
    ] {
        for body in [
            format!("a = {expression}\nb = {expression}\n"),
            format!("a = {{{expression}, {expression}}}\n"),
            format!("a = (({expression},), ({expression},))\n"),
            format!("a = (({expression},) * 2,) * 2\n"),
            format!("a = (+({expression}), -({expression}))\n"),
            format!("a = ({expression},)[0]\nb = {expression}\n"),
            format!(
                "def first():\n    return {expression}\ndef second():\n    return {expression}\n"
            ),
        ] {
            for prefix in ["", indexed_prefix.as_str()] {
                common::assert_matches_cpython_marshal(
                    &format!("{prefix}{body}"),
                    "nan_identity.py",
                );
            }
        }
    }
}

#[test]
fn executes_distinct_and_repeated_nan_constants() {
    common::assert_executes(
        "a = 1e999 - 1e999\nb = 1e999 - 1e999\n\
         print(a is b)\n\
         print(len({1e999 - 1e999, 1e999 - 1e999}))\n\
         pair = (1e999 - 1e999,) * 2\n\
         print(pair[0] is pair[1], len(set(pair)))\n\
         nested = ((1e999 - 1e999,),) * 2\n\
         print(nested[0] is nested[1], nested[0][0] is nested[1][0])\n\
         print(len({1e999j - 1e999j, 1e999j - 1e999j}))\n",
        "False\n2\nTrue 1\nTrue True\n2\n",
    );
}

#[test]
fn nan_marshal_is_stable_across_compilations() {
    let source = "values = {(1e999 - 1e999, 1), (1e999 - 1e999, 2)}\n\
                  repeated = ((1e999j - 1e999j,),) * 2\n";
    let first = serc::compile(source, "nan_stability.py").unwrap();
    for _ in 0..8 {
        let second = serc::compile(source, "nan_stability.py").unwrap();
        assert_eq!(first.marshal(), second.marshal());
    }
    common::assert_matches_cpython_marshal(source, "nan_stability.py");
}
