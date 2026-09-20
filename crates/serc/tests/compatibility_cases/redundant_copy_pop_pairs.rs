use crate::common;
use crate::common::indent;

#[test]
fn matches_cpython_redundant_local_assignment_copies() {
    for assignment in [
        "x = x = value\n",
        "x = x = 7\n",
        "x = x = x = value\n",
        "(\n    x\n) = (\n    x\n) = value\n",
    ] {
        for wrapper in ["plain", "try", "with", "finally"] {
            for assertion in [false, true] {
                let mut body = match wrapper {
                    "plain" => assignment.to_owned(),
                    "try" => format!("try:\n{}except Error:\n    report()\n", indent(assignment)),
                    "with" => format!("with manager:\n{}", indent(assignment)),
                    "finally" => format!("try:\n{}finally:\n    cleanup()\n", indent(assignment)),
                    _ => unreachable!(),
                };
                if assertion {
                    body.push_str("assert value\n");
                }
                body.push_str("return value\n");
                let source = format!("def f(value, manager):\n{}", indent(&body));
                common::assert_matches_cpython_marshal(&source, "redundant_local_copies.py");
            }
        }
    }
}

#[test]
fn matches_cpython_guarded_wildcard_copies() {
    for (asynchronous, literal_case, protected) in [
        (false, false, false),
        (false, true, false),
        (false, false, true),
        (true, false, false),
        (true, false, true),
    ] {
        for case_body in ["consume(value)\n", "result = value\n"] {
            let mut body = String::from("match value:\n");
            if literal_case {
                body.push_str("    case 1:\n");
                body.push_str(&indent(&indent(case_body)));
            }
            for pattern in ["case _ if flag:", "case _ if other(value):", "case _:"] {
                body.push_str("    ");
                body.push_str(pattern);
                body.push('\n');
                body.push_str(&indent(&indent(case_body)));
            }
            if protected {
                body = format!("with manager:\n{}", indent(&body));
            }
            body.push_str("return value\n");
            let prefix = if asynchronous { "async " } else { "" };
            let source = format!("{prefix}def f(value, manager, flag):\n{}", indent(&body));
            common::assert_matches_cpython_marshal(&source, "guarded_wildcard_copies.py");
        }
    }
}
