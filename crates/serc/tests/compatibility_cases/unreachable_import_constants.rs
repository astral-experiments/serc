use crate::common;

#[test]
fn matches_cpython_first_constant_in_unreachable_imports() {
    for import in [
        "import package",
        "import package.module as alias",
        "from package import item",
        "from ..package import item",
    ] {
        for prefix in ["", "    'doc'\n", "    marker = True\n"] {
            let source =
                format!("def f():\n{prefix}    raise Error\n    {import}\n    marker = False\n");
            common::assert_matches_cpython_marshal(&source, "unreachable_import_constant.py");
        }
        for tail in [
            format!("    marker = False\n    {import}\n"),
            format!("    {import}\n"),
            format!("    if condition:\n        {import}\n    marker = False\n"),
        ] {
            let source = format!("def f():\n    raise Error\n{tail}");
            common::assert_matches_cpython_marshal(&source, "unreachable_import_order.py");
        }
    }
}
