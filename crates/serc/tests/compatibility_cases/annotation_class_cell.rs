use crate::common;

#[test]
fn annotation_scopes_receive_the_current_class_cell() {
    for definition in [
        "field: __class__",
        "def method(arg: __class__): pass",
        "type Alias = __class__",
        "type Alias[T: __class__] = T",
        "def method[T: __class__](): pass",
        "def method[T: lambda: __class__](): pass",
        "class Inner[T: lambda: __class__]: pass",
        "class Inner[T](__class__): pass",
        "class Inner[T](metaclass=__class__): pass",
        "class Inner[T](factory(lambda: __class__)): pass",
        "class Inner[__class__](__class__): pass",
        "def method[T = __class__](): pass",
    ] {
        let source = format!("class C:\n    {definition}\n");
        common::assert_matches_cpython_marshal(&source, "annotation_class_cell.py");
        let source =
            format!("def outer(__class__):\n    class C:\n        {definition}\n    return C\n");
        common::assert_matches_cpython_marshal(&source, "nested_annotation_class_cell.py");
    }
    common::assert_executes(
        "class C:\n field: __class__\n def method(arg: __class__): pass\n type Alias = __class__\n def generic[T: lambda: __class__](): pass\nprint(C.__annotations__['field'] is C)\nprint(C.method.__annotations__['arg'] is C)\nprint(C.Alias.__value__ is C)\nprint(C.generic.__type_params__[0].__bound__() is C)\n",
        "True\nTrue\nTrue\nTrue\n",
    );
}

#[test]
fn annotation_class_cells_respect_shadowing_and_postponed_annotations() {
    for declaration in ["__class__ = int", "global __class__"] {
        for definition in [
            "field: __class__",
            "def method(arg: __class__): pass",
            "type Alias = __class__",
            "def method[T: __class__](): pass",
            "def method[T: lambda: __class__](): pass",
            "type Alias[T] = lambda: __class__",
        ] {
            let source = format!("class C:\n    {declaration}\n    {definition}\n");
            common::assert_matches_cpython_marshal(&source, "annotation_class_cell_shadowing.py");
        }
    }
    common::assert_matches_cpython_marshal(
        "from __future__ import annotations\nclass C:\n field: __class__\n def method(arg: __class__): pass\n",
        "postponed_class_cell.py",
    );
}

#[test]
fn annotation_class_cells_do_not_replace_class_body_outer_captures() {
    let source = "def outer(__class__):\n class C:\n  before = __class__\n  field: __class__\n  def method(arg: __class__): pass\n return C\nC = outer(42)\nprint(C.before)\nprint(C.__annotations__['field'] is C)\nprint(C.method.__annotations__['arg'] is C)\n";
    common::assert_matches_cpython_marshal(source, "annotation_and_outer_class_cells.py");
    common::assert_executes(source, "42\nTrue\nTrue\n");
}

#[test]
fn nested_annotation_class_cells_bypass_class_bindings() {
    for declaration in ["__class__ = int", "global __class__"] {
        for expression in [
            "lambda: __class__",
            "(__class__, lambda: __class__)",
            "[__class__ for _ in (0,)]",
            "[lambda: __class__ for _ in (0,)]",
        ] {
            let source =
                format!("class C:\n    {declaration}\n    def method(arg: {expression}): pass\n");
            common::assert_matches_cpython_marshal(&source, "nested_annotation_class_cells.py");
            common::assert_matches_cpython_marshal(
                &format!("from __future__ import annotations\n{source}"),
                "postponed_nested_annotation_class_cells.py",
            );
        }
    }
    common::assert_executes(
        "__class__ = 'module'\nclass Bound:\n __class__ = int\n def method(arg: (__class__, lambda: __class__, [__class__ for _ in (0,)])): pass\nclass Global:\n global __class__\n def method(arg: (__class__, lambda: __class__, [__class__ for _ in (0,)])): pass\nfor cls, expected in ((Bound, int), (Global, __class__)):\n direct, nested, comprehension = cls.method.__annotations__['arg']\n print(direct is expected, nested() is cls, comprehension == [cls])\n",
        "True True True\nTrue True True\n",
    );
}

#[test]
fn annotation_type_parameters_shadow_an_existing_class_cell() {
    for name in ["T", "__class__"] {
        for expression in [
            name.to_string(),
            format!("lambda: {name}"),
            format!("({name}, lambda: {name})"),
            format!("[{name} for _ in (0,)]"),
        ] {
            let source = format!(
                "class C:\n    {name} = int\n    def method[{name}](arg: {expression}): pass\n    def seeded(self): return __class__\n"
            );
            common::assert_matches_cpython_marshal(&source, "annotation_class_type_parameter.py");
        }
    }
    common::assert_executes(
        "class C:\n __class__ = int\n def method[__class__](arg: lambda: __class__): pass\n def seeded(self): return __class__\nprint(C.method.__annotations__['arg']() is C.method.__type_params__[0])\nprint(C.seeded(None) is C)\n",
        "True\nTrue\n",
    );
}

#[test]
fn annotation_comprehensions_do_not_replace_class_body_outer_captures() {
    for (definition, annotation) in [
        (
            "field: [__class__ for _ in (0,)]",
            "C.__annotations__['field']",
        ),
        (
            "def method(arg: [__class__ for _ in (0,)]): pass",
            "C.method.__annotations__['arg']",
        ),
    ] {
        let source = format!(
            "def outer(__class__):\n class C:\n  before = __class__\n  {definition}\n return C\nC = outer(42)\nprint(C.before)\nprint({annotation} == [C])\n"
        );
        common::assert_matches_cpython_marshal(&source, "annotation_comprehension_outer_class.py");
        common::assert_executes(&source, "42\nTrue\n");
    }
}

#[test]
fn class_bound_type_parameters_preserve_nested_annotation_captures() {
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
            let source = format!("class C:\n    T = int\n    {definition}\n");
            common::assert_matches_cpython_marshal(&source, "class_bound_type_parameters.py");
        }
    }
    common::assert_executes(
        "class C:\n T = int\n type Alias[T] = (T, lambda: T, [T for _ in (0,)])\nprint(C.Alias.__value__[0] is int)\nprint(C.Alias.__value__[1]() is C.Alias.__type_params__[0])\nprint(C.Alias.__value__[2] == [C.Alias.__type_params__[0]])\n",
        "True\nTrue\nTrue\n",
    );
}
