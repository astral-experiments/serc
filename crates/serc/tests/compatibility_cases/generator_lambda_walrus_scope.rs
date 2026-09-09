use crate::common;

#[test]
fn keeps_lambda_body_walruses_local_inside_generators() {
    for value in [
        "(lambda: (assigned := item))",
        "(lambda: (assigned := [element for element in values]))",
        "(lambda: (assigned := (element for element in values)))",
        "(lambda: (lambda: (assigned := item)))",
        "(lambda default=(assigned := item): default)",
        "(lambda *, default=(assigned := item): (assigned := default))",
    ] {
        for (prefix, declaration) in [
            ("", "    assigned = None\n"),
            ("", "    global assigned\n"),
            (
                "def enclosing(values):\n    assigned = None\n",
                "    nonlocal assigned\n",
            ),
        ] {
            let function = format!(
                "def function(values):\n{declaration}    result = consume({value} for item in values)\n    return assigned, result\n"
            );
            let source = if prefix.is_empty() {
                function
            } else {
                format!("{prefix}    {}", function.replace('\n', "\n    "))
            };
            common::assert_matches_cpython_marshal(&source, "generator_lambda_walrus_scope.py");
        }
    }
}
