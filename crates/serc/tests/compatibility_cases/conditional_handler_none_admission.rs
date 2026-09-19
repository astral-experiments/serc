use crate::common;

#[test]
fn matches_cpython_named_handler_cleanup_none_admission() {
    for (definition, prefix) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield flag\n"),
        ("async def", "    yield flag\n"),
    ] {
        for body in [
            "        if flag:\n            consume('left')\n        else:\n            consume('right')\n",
            "        if flag:\n            consume('left')\n        elif other:\n            consume('middle')\n        else:\n            consume('right')\n",
            "        if flag:\n            return\n        else:\n            consume('right')\n",
            "        if flag:\n            consume('left')\n        else:\n            return\n",
        ] {
            for following_handler in ["", "    except Other:\n        consume('after_handler')\n"] {
                let source = format!(
                    "{definition} f(flag, other):\n{prefix}    try:\n        consume('before')\n    except Error as exc:\n{body}{following_handler}"
                );
                common::assert_matches_cpython_marshal(
                    &source,
                    "named_handler_cleanup_none_admission.py",
                );
            }
        }
    }
}
