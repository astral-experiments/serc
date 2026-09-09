use crate::common;

#[test]
fn matches_cpython_terminating_handler_shared_false_join_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield a\n"),
        ("async def", "    yield a\n"),
    ] {
        // Repeated operands still emit separate edges into the original cleanup block.
        for condition in [
            "a and b",
            "a or (b and c)",
            "not (a or b)",
            "a and a",
            "a or (b and b)",
        ] {
            for action in ["raise", "return"] {
                let source = format!(
                    "{function} f(a, b, c):\n{suspension}    try:\n        work()\n    except Exception as error:\n        if {condition}:\n            {action}\n    after()\n"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "terminating_handler_shared_false_join_locations.py",
                );
            }
        }
    }
}

#[test]
fn matches_cpython_terminating_handler_single_false_edge_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield a\n"),
        ("async def", "    yield a\n"),
    ] {
        for condition in ["a or b", "a"] {
            let source = format!(
                "{function} f(a, b):\n{suspension}    try:\n        work()\n    except Exception as error:\n        if {condition}:\n            raise\n    after()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "terminating_handler_single_false_edge_locations.py",
            );
        }
    }
}

#[test]
fn matches_cpython_terminating_handler_shared_false_join_before_else() {
    for condition in ["a or (b and c)", "not (a or b)"] {
        for action in ["raise", "return"] {
            let source = format!(
                "async def f(a, b, c):\n    try:\n        work()\n    except Exception as error:\n        if {condition}:\n            {action}\n    else:\n        observe()\n    after()\n"
            );
            common::assert_matches_cpython_marshal(
                &source,
                "terminating_handler_shared_false_join_before_else.py",
            );
        }
    }
}
