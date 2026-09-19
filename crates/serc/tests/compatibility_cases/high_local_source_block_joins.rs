use std::fmt::Write as _;

use crate::common;

#[test]
fn high_locals_reset_at_source_joins_and_preserve_copied_continuations() {
    for padding in [60, 61, 62, 70] {
        for prior_handler in [false, true] {
            for body in [
                "if flag:\n    return None\nelse:\n    selected = value\nreturn selected\n",
                "if flag:\n    selected = value\nelse:\n    selected = other\nreturn selected\n",
                "if flag:\n    selected = value\n    assert False, selected\nreturn None\n",
                "while True:\n    if flag:\n        return None\n    selected = value\n    break\nreturn selected\n",
                "selected = value\ntry:\n    consume(selected)\nexcept Error:\n    pass\nreturn selected\n",
                "selected = value\nwith manager():\n    consume(selected)\nreturn selected\n",
                "selected = value\nwhile True:\n    consume(selected)\n    break\nconsume(selected)\nreturn selected\n",
                "selected = value\nfor item in other:\n    if item:\n        break\nelse:\n    consume(selected)\nreturn selected\n",
            ] {
                let mut source = String::from("def f(flag, value, other):\n");
                if prior_handler {
                    source.push_str("    try:\n        work()\n    except Error:\n        pass\n");
                }
                for index in 0..padding {
                    writeln!(source, "    local{index} = {index}").unwrap();
                }
                for line in body.lines() {
                    writeln!(source, "    {line}").unwrap();
                }
                common::assert_matches_cpython_marshal(&source, "high_local_source_block_joins.py");
            }
        }
    }
}
