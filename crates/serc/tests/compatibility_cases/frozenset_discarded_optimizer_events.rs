use crate::common;

fn check_declaration(declaration: &str) {
    let consumers = "def first(value):\n    return value in {'_Container__key', 'novel_marker'}\ndef second(value):\n    'Distinct constants tuple.'\n    return value in {'_Container__key', 'novel_marker'}\n";
    let mut seed = "if False:\n".to_string();
    for line in declaration.lines() {
        seed.push_str("    ");
        seed.push_str(line);
        seed.push('\n');
    }
    for before in [false, true] {
        for prefix in ["", "prior = '_Container__key'\n"] {
            let source = if before {
                format!("{prefix}{seed}{consumers}")
            } else {
                format!("{prefix}{consumers}{seed}")
            };
            common::assert_matches_cpython_marshal(&source, "discarded_optimizer_events.py");
        }
    }
}

#[test]
fn matches_cpython_discarded_folded_constant_assembly() {
    for expression in [
        "'_Container' + '__key'",
        "'_Container__key' + 'suffix'",
        "'x' + '_Container__key'",
        "('_Container__key', 1)[0]",
        "('_Container__key', 1)[1]",
        "('_Container__key',) * 0",
    ] {
        for declaration in [
            format!("class Container:\n    def seed(__key):\n        return {expression}\n"),
            format!("class Container:\n    seed = lambda __key: {expression}\n"),
            format!(
                "class Container:\n    def seed(__key):\n        def inner():\n            pass\n        return {expression}\n"
            ),
            format!(
                "class Container:\n    def seed(__key):\n        value = None\n        return {expression}\n"
            ),
            format!(
                "class Container:\n    def seed(__key):\n        value = None\n    type Alias = {expression}\n"
            ),
            format!("class Container[T: {expression}]:\n    def seed(__key):\n        pass\n"),
        ] {
            check_declaration(&declaration);
        }
    }
}

#[test]
fn matches_cpython_discarded_nested_lambda_and_later_private_parameter() {
    for declaration in [
        "class Container:\n    seed = lambda __key: lambda: (__key, '_Container__key')\n",
        "class Container:\n    seed = lambda __key: (lambda: __key, '_Container__key')\n",
        "class Container[T: (__key, '_Container__key'), __key]:\n    pass\n",
        "class Container[T: (lambda: (__key, '_Container__key')), __key]:\n    pass\n",
    ] {
        check_declaration(declaration);
    }
}

#[test]
fn matches_cpython_optimizer_tuple_admission_before_discarded_assembly() {
    for result in [
        "('_Container__key',)",
        "('_Container__key' if flag else '_Container__key',)",
        "('_Container__key' if flag else 'other',)",
        "('other' if flag else '_Container__key',)",
    ] {
        for discarded in [false, true] {
            let declaration = format!(
                "class Container:\n    def seed(flag):\n        __key\n        value = {{'_Container__key', 'novel_marker'}}\n        return {result}\n"
            );
            let seed = if discarded {
                let mut seed = "if False:\n".to_string();
                for line in declaration.lines() {
                    seed.push_str("    ");
                    seed.push_str(line);
                    seed.push('\n');
                }
                seed
            } else {
                declaration
            };
            let source = format!(
                "{seed}def first(value):\n    return value in {{'_Container__key', 'novel_marker'}}\ndef second(value):\n    'Distinct constants tuple.'\n    return value in {{'_Container__key', 'novel_marker'}}\n"
            );
            common::assert_matches_cpython_marshal(&source, "optimizer_tuple_admission.py");
        }
    }
}
