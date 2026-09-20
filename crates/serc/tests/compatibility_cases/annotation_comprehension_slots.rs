use crate::common;

#[test]
fn matches_cpython_annotation_comprehension_slots() {
    for expression in [
        "[probe(x) for x in xs]",
        "[probe for probe in probe]",
        "[([probe for probe in ys], probe(x)) for x in xs]",
        "[probe(y) for x in xs for y in [probe for probe in ys]]",
    ] {
        for (parameters, returns) in [
            (format!("arg: {expression}"), String::new()),
            ("arg".to_string(), format!(" -> {expression}")),
            (
                format!("first: {expression}, second: [other for other in ys]"),
                " -> [last for last in xs]".to_string(),
            ),
        ] {
            let source = format!(
                "def outer(probe, xs, ys):\n    def child({parameters}){returns}:\n        return 0\n    return child\n"
            );
            common::assert_matches_cpython_marshal(&source, "annotation_comprehension_slots.py");
        }
    }
    common::assert_matches_cpython_marshal(
        "def outer(probe):\n    async def f(xs, ys, funcs):\n        def child(arg: [([probe for probe in ys], probe(x)) for x in xs]):\n            return arg\n        second = [probe for probe in ys]\n        return child, second\n    return f\n",
        "annotation_nested_comprehension_slots.py",
    );
    for source in [
        "def outer(xs):\n    def child(arg: [format(x) for x in xs]):\n        return arg\n    return child\n",
        "class Holder:\n    def method(self, __probe, xs):\n        def child(arg: [__probe(x) for x in xs]):\n            return arg\n        return child\n",
        "class Holder:\n    def method(self, __probe, xs, ys):\n        def child(arg: [([__probe for __probe in ys], __probe(x)) for x in xs]):\n            return arg\n        return child\n",
    ] {
        common::assert_matches_cpython_marshal(source, "annotation_comprehension_names.py");
    }
}

#[test]
fn preserves_stringized_annotation_comprehensions() {
    for expression in [
        "[lambda: probe for probe in xs]",
        "[(lambda saved=[probe for probe in ys]: saved)() for x in xs]",
        "[(probe(x) for x in xs), [probe for probe in ys]]",
        "[probe for probe in probe]",
    ] {
        for asynchronous in [false, true] {
            let prefix = if asynchronous { "async " } else { "" };
            let source = format!(
                "from __future__ import annotations\ndef outer(probe, xs, ys):\n    {prefix}def child(arg: {expression}):\n        return 0\n    return child\n"
            );
            common::assert_matches_cpython_marshal(&source, "annotation_comprehension_strings.py");
        }
    }
}

#[test]
fn conditional_annotations_preserve_unprotected_and_protected_code() {
    // The plain annotation has no cleanup; the inline comprehension must restore its local on
    // failure. The protected function's shared conditional tails also retain their original owner.
    let source = r"def plain(value: left if gate else right) -> (left and right):
    return value

def protected(value: [candidate if gate else fallback
                      for candidate in values if keep(candidate)]):
    try:
        if outer:
            if left or right:
                consume(value)
            else:
                consume(None)
        else:
            if left or right:
                consume(None)
            else:
                consume(value)
    except ValueError:
        cleanup()
    return value
";
    common::assert_matches_cpython_marshal(source, "conditional_annotation_owners.py");
}
