use crate::common;

#[test]
fn matches_cpython_function_global_comprehension_save_order() {
    for body in [
        "[saved := x for x in values]",
        "{saved := x for x in values}",
        "{(saved := x): x for x in values}",
        "{x: (saved := x) for x in values}",
        "[x for x in values if (saved := x)]",
        "[saved := (second := x) for x in values]",
        "[[saved := y for y in values] for x in values]",
        "{(saved := x): [y for y in values] for x in values}",
        "[(saved, [saved := y for y in values]) for x in values]",
        "saved = 0\n[saved := x for x in values]",
        "[saved := x for x in values]\nlocal = 0",
        "local = 0\n[saved := x for x in values]",
        "local = 0\nreader = lambda: local\n[saved := x for x in values]",
        "[saved := x for x in values]\nreader = lambda: saved",
    ] {
        let mut source = String::from("def f(values):\n    global saved, second\n");
        for line in body.lines() {
            source.push_str("    ");
            source.push_str(line);
            source.push('\n');
        }
        source.push_str("    return saved\n");
        common::assert_matches_cpython_marshal(&source, "function_global_comprehension.py");
    }
}

#[test]
fn matches_cpython_private_global_comprehension_aliases() {
    for body in [
        "global __saved\n[_C__saved := x for x in values]",
        "global _C__saved\n[__saved := x for x in values]",
        "global __saved\n[_C__saved := x for x in values]\n[__saved := x for x in values]",
        "global __saved\n[(__saved := x, _C__saved := x) for x in values]",
        "global __saved\n[(__saved, [_C__saved := y for y in values]) for x in values]",
        "global __saved, second\nreturn [(_C__saved, (second := 1), (__saved := 2)) for x in values]",
        "global __saved\n_C__saved = _C__saved + 1",
        "global _C__saved\n__saved = __saved + 1",
    ] {
        let mut source = String::from("class C:\n    def f(self, values):\n");
        for line in body.lines() {
            source.push_str("        ");
            source.push_str(line);
            source.push('\n');
        }
        common::assert_matches_cpython_marshal(&source, "private_global_comprehension.py");
    }
}

#[test]
fn matches_cpython_private_global_comprehension_cell_aliases() {
    for body in [
        "[__saved := x for x in values]\n[_C__saved for _C__saved in values]",
        "[_C__saved for _C__saved in values]\n[lambda: __saved for __saved in values]",
        "[__saved for __saved in values]\n[lambda: _C__saved for _C__saved in values]",
        "[__saved := x for x in values]\n[lambda: _C__saved for _C__saved in values]",
    ] {
        let mut source =
            String::from("class C:\n    def f(self, values):\n        global __saved\n");
        for line in body.lines() {
            source.push_str("        ");
            source.push_str(line);
            source.push('\n');
        }
        common::assert_matches_cpython_marshal(&source, "private_global_comprehension_cells.py");
    }
}

#[test]
fn matches_cpython_global_comprehension_scope_boundaries() {
    for source in [
        "def f(values):\n    global saved\n    (saved := x for x in values)\n    return saved\n",
        "def f(values):\n    global saved\n    lambda: [saved := x for x in values]\n    return saved\n",
        "def outer(values):\n    saved = 0\n    def f():\n        nonlocal saved\n        [saved := x for x in values]\n        return saved\n",
        "def f[T](values):\n    global saved\n    [saved := x for x in values]\n    return saved\n",
        "[saved := x for x in values]\n",
    ] {
        common::assert_matches_cpython_marshal(source, "global_comprehension_scope.py");
    }
}
