use crate::common;

fn function_source(scope: &str, body: &str) -> String {
    let declaration = if matches!(scope, "async" | "agen") {
        "async def"
    } else {
        "def"
    };
    let mut source = format!("{declaration} function(left, right, flag, consume, work):\n");
    if matches!(scope, "async" | "agen") {
        source.push_str("    await work(flag)\n");
    }
    if matches!(scope, "gen" | "agen") {
        source.push_str("    yield flag\n");
    }
    for line in body.lines() {
        source.push_str("    ");
        source.push_str(line);
        source.push('\n');
    }
    source
}

#[test]
fn preserves_false_arm_borrow_entries_in_values_and_returns() {
    for scope in ["sync", "async", "gen", "agen"] {
        for condition in ["False", "None", "0"] {
            let expression = format!("(left if {condition} else right)");
            let returned = if scope == "agen" {
                format!("yield {expression}\nreturn")
            } else {
                format!("return {expression}")
            };
            let tail = if scope == "agen" {
                "yield right\nreturn"
            } else {
                "return right"
            };
            for body in [
                returned,
                format!("consume(flag, {expression}, right)\n{tail}"),
                format!("result = {expression}\nconsume(result, right)\n{tail}"),
            ] {
                common::assert_matches_cpython_marshal(
                    &function_source(scope, &body),
                    "folded_false_value_entry.py",
                );
            }
        }
    }
}

#[test]
fn preserves_false_arm_borrow_entries_in_conditions() {
    // Generator conditions retain an independent NOT_TAKEN exception-owner difference;
    // the paired compiler matrix checks their loads while these cases require full bytes.
    for scope in ["sync", "async", "agen"] {
        for condition in ["False", "None", "0"] {
            let tail = if scope == "agen" {
                "yield right\nreturn"
            } else {
                "return right"
            };
            let body = format!("if (left if {condition} else right):\n    consume(flag)\n{tail}");
            common::assert_matches_cpython_marshal(
                &function_source(scope, &body),
                "folded_false_condition_entry.py",
            );
        }
    }
}

#[test]
fn preserves_true_arm_assignment_borrow_entries() {
    for scope in ["sync", "async", "gen", "agen"] {
        for condition in ["True", "1"] {
            let tail = if scope == "agen" {
                "yield right\nreturn"
            } else {
                "return right"
            };
            let body = format!(
                "result = (left if {condition} else right)\nconsume(result, right)\n{tail}"
            );
            common::assert_matches_cpython_marshal(
                &function_source(scope, &body),
                "folded_true_assignment_entry.py",
            );
        }
    }
}
