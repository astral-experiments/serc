use crate::common;

#[test]
fn class_private_global_aliases_match_cpython() {
    for body in [
        "global __value\nresult = _C__value",
        "global _C__value\nresult = __value",
        "global __value\n_C__value = 1\ndel _C__value",
        "global _C__value\n__value = 1\ndel __value",
        "global __value\nresult = _C__value\nreader = lambda: _C__value",
        "global __value\nresult = [_C__value for _ in range(1)]",
        "global __value\ndef _C__value(): pass",
        "global _C__value\ndef __value(): pass",
    ] {
        let source = format!(
            "def outer():\n    _C__value = 42\n    class C:\n{}    return C\n",
            common::indent(&common::indent(body))
        );
        common::assert_matches_cpython_marshal(&source, "class_private_global.py");
    }
}

#[test]
fn class_private_global_aliases_read_and_mutate_module_bindings() {
    let source = r"
_C__value = 'global'
def outer():
    _C__value = 'outer'
    class C:
        global __value
        result = _C__value
        _C__value = 'updated'
    return C.result
print(outer(), _C__value)
";
    common::assert_executes(source, "global updated\n");
    common::assert_matches_cpython_marshal(source, "class_private_global.py");
}
