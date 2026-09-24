use crate::common::{assert_executes, assert_matches_cpython_marshal};

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

#[test]
fn class_annotations_look_up_dunder_classdict_in_the_class_namespace() {
    let source = r#"class Shadow:
    locals()["__classdict__"] = "shadow"
    field: __classdict__
    def method(value: __classdict__) -> __classdict__: pass
    def generic[T: __classdict__](value: __classdict__): pass
    type Alias = __classdict__

class Fallback:
    marker = 42
    field: __classdict__
    def method(value: __classdict__): pass
    type Alias = __classdict__
"#;
    assert_matches_cpython_marshal(source, "class_annotations_dunder_classdict.py");
    assert_executes(
        &format!(
            "{source}\n\
             print(Shadow.__annotations__['field'], Shadow.method.__annotations__, Shadow.Alias.__value__)\n\
             print(Shadow.generic.__annotations__, Shadow.generic.__type_params__[0].__bound__)\n\
             print(Fallback.__annotations__['field']['marker'], Fallback.method.__annotations__['value']['marker'], Fallback.Alias.__value__['marker'])\n"
        ),
        "shadow {'value': 'shadow', 'return': 'shadow'} shadow\n{'value': 'shadow'} shadow\n42 42 42\n",
    );
}

#[test]
fn class_annotations_respect_explicit_dunder_classdict_global() {
    let source = r#"__classdict__ = "global"
class C:
    global __classdict__
    locals()["__classdict__"] = "shadow"
    field: __classdict__
    def method(value: __classdict__): pass
    def generic[T: __classdict__](value: __classdict__): pass
    type Alias = __classdict__
"#;
    assert_executes(
        &format!(
            "{source}\n\
             print(C.__annotations__, C.method.__annotations__, C.generic.__annotations__, C.generic.__type_params__[0].__bound__, C.Alias.__value__)\n"
        ),
        "{'field': 'global'} {'value': 'global'} {'value': 'global'} global global\n",
    );
}
