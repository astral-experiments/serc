use crate::common;

#[test]
fn type_parameter_scopes_honor_explicit_class_globals() {
    for definition in [
        "type Alias = x",
        "type Alias = x()",
        "type Alias[T] = x",
        "type Alias[T: x] = T",
        "type Alias[T = x] = T",
        "def method[T: x](): pass",
        "def method[T = x](): pass",
        "class Inner[T: x]: pass",
        "class Inner[T = x]: pass",
    ] {
        let source =
            format!("x = int\nclass C:\n    global x\n    locals()['x'] = str\n    {definition}\n");
        common::assert_matches_cpython_marshal(&source, "type_parameter_globals.py");
    }
    common::assert_executes(
        "x = int\nclass C:\n global x\n locals()['x'] = str\n type Alias = x\n def method[T: x](): pass\n class Inner[T = x]: pass\nprint(C.Alias.__value__ is int)\nprint(C.method.__type_params__[0].__bound__ is int)\nprint(C.Inner.__type_params__[0].__default__ is int)\n",
        "True\nTrue\nTrue\n",
    );
}

#[test]
fn explicit_class_globals_do_not_hide_nested_type_parameter_captures() {
    for expression in [
        "T",
        "lambda: T",
        "(T, lambda: T)",
        "[T for _ in (0,)]",
        "(T, [T for _ in (0,)])",
        "[T for T in (0,)]",
    ] {
        for definition in [
            format!("type Alias[T] = {expression}"),
            format!("def method[T: {expression}](): pass"),
            format!("class Inner[T: {expression}]: pass"),
        ] {
            let source = format!("T = int\nclass C:\n    global T\n    {definition}\n");
            common::assert_matches_cpython_marshal(&source, "type_parameter_global_capture.py");
        }
    }
    common::assert_executes(
        "T = int\nclass C:\n global T\n type Alias[T] = (T, lambda: T, [T for _ in (0,)])\nprint(C.Alias.__value__[0] is int)\nprint(C.Alias.__value__[1]() is C.Alias.__type_params__[0])\nprint(C.Alias.__value__[2] == [C.Alias.__type_params__[0]])\n",
        "True\nTrue\nTrue\n",
    );
}
