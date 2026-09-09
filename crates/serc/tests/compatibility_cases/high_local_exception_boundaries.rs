use std::fmt::Write;

use crate::common;

#[test]
fn checks_high_comprehension_results_after_branches() {
    for count in [60, 62, 63, 70] {
        let mut assignments = String::new();
        for index in 0..count {
            writeln!(assignments, "    local{index} = {index}").unwrap();
        }
        for producer in [
            "[item for item in values]",
            "{item for item in values}",
            "{item: item for item in values}",
        ] {
            for condition in [
                "any(item for item in result)",
                "tuple(item for item in result)",
                "flag and result",
                "flag or result",
            ] {
                for protected in [false, true] {
                    let body = format!(
                        "result = {producer}\nif {condition}:\n    consume(result)\nreturn result\n"
                    );
                    let body = if protected {
                        format!(
                            "try:\n    {}\nexcept Error:\n    return None",
                            body.trim_end().replace('\n', "\n    ")
                        )
                    } else {
                        body
                    };
                    let source = format!(
                        "def f(values, flag, other):\n{assignments}    {}\n",
                        body.trim_end().replace('\n', "\n    ")
                    );
                    common::assert_matches_cpython_marshal(&source, "high_local_branch_checks.py");
                }
            }
        }
    }
}

#[test]
fn matches_cpython_high_locals_across_exception_boundaries() {
    for count in [61, 62, 63, 70] {
        let mut assignments = String::new();
        for index in 0..count {
            writeln!(assignments, "    local{index} = {index}").unwrap();
        }
        for body in [
            "    selected = make()\n    with manager() as handle:\n        handle.consume(selected)\n",
            "    selected = make()\n    try:\n        consume(selected)\n    except Error:\n        pass\n",
            "    selected = make()\n    if flag:\n        consume(selected)\n",
            "    selected = make()\n    with manager():\n        del selected\n        consume(selected)\n",
            "    selected = [value for value in items]\n    return consume(value for value in selected)\n",
        ] {
            let source = format!("def f(items, flag):\n{assignments}{body}");
            common::assert_matches_cpython_marshal(&source, "high_local_regions.py");
        }
    }
}
