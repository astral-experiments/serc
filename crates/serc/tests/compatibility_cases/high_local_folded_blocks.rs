use std::fmt::Write as _;

use crate::common;

#[test]
fn folded_conditions_preserve_local_blocks_and_copyable_joins() {
    for padding in [60, 61] {
        for protected in [false, true] {
            for body in [
                "assert True\nconsume(h)\n",
                "if True:\n    consume(h)\nelse:\n    consume(value)\n",
                "if False:\n    consume(value)\nelse:\n    consume(h)\n",
                "result = h if True else value\nconsume(h)\n",
            ] {
                for tail in ["return h\n", "return h, value\n", "return h, value, flag\n"] {
                    let mut source = String::from("def f(value, flag, items):\n");
                    for index in 0..padding {
                        writeln!(source, "    local{index} = {index}").unwrap();
                    }
                    source.push_str("    h = value\n");
                    if protected {
                        source.push_str("    try:\n");
                        for line in body.lines() {
                            writeln!(source, "        {line}").unwrap();
                        }
                        source.push_str("    except Error:\n        pass\n");
                    } else {
                        source.push_str(
                            "    try:\n        prepare(value)\n    except Error:\n        pass\n",
                        );
                        for line in body.lines() {
                            writeln!(source, "    {line}").unwrap();
                        }
                    }
                    for line in tail.lines() {
                        writeln!(source, "    {line}").unwrap();
                    }
                    common::assert_matches_cpython_marshal(&source, "high_local_folded_blocks.py");
                }
            }
        }
    }
}
