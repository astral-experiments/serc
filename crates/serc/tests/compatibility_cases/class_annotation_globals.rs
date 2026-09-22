use crate::common::assert_executes;

#[test]
fn class_annotations_respect_explicit_globals() {
    assert_executes(
        "x = 'global'\nclass C:\n global x\n locals()['x'] = 'class'\n field: x\n def f(arg: x) -> x: pass\nprint(C.__annotations__)\nprint(C.f.__annotations__)\n",
        "{'field': 'global'}\n{'arg': 'global', 'return': 'global'}\n",
    );
    assert_executes(
        "T = 'global'\nclass C:\n global T\n locals()['T'] = 'class'\n def f[T](arg: T): pass\nprint(C.f.__annotations__)\n",
        "{'arg': 'global'}\n",
    );
}
