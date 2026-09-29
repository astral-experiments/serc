use std::io::Write;
use std::process::{Command, Stdio};

use serc::PythonVersion;

#[path = "common/private_type_parameters.rs"]
mod cpython;

use cpython::{assert_matches_cpython, targets};

#[test]
fn private_type_parameter_aliases_match_cpython() {
    let cases = [
        (
            "nested_header_keeps_raw_enclosing_capture",
            false,
            r"def outer(__T):
    class Outer[__T]:
        class Inner[U: __T]:
            pass
    return Outer
assert outer(42).Inner.__type_params__[0].__bound__ == 42
",
        ),
        (
            "nested_lambda_bypasses_class_namespace_alias",
            true,
            r"class Outer[__T]:
    _Outer__T = 42
    class Inner[U: (lambda: _Outer__T)]:
        pass
assert Outer.Inner.__type_params__[0].__bound__() is Outer.__type_params__[0]
",
        ),
        (
            "nested_header_uses_own_parameter_before_enclosing_alias",
            false,
            r"def outer(_Outer__T):
    class Outer[__T]:
        class Inner[U: _Outer__T]:
            pass
    return Outer
Outer = outer(42)
assert Outer.Inner.__type_params__[0].__bound__ is Outer.__type_params__[0]
",
        ),
        (
            "nested_header_preserves_class_namespace_precedence",
            false,
            r"class Outer[__T]:
    _Outer__T = 42
    class Inner[U: _Outer__T]:
        pass
assert Outer.Inner.__type_params__[0].__bound__ == 42
",
        ),
        (
            "ordinary_header_reads_enclosing_private_parameter",
            false,
            r"class Outer[__T]:
    class Inner[U: _Outer__T]:
        pass
assert Outer.Inner.__type_params__[0].__bound__ is Outer.__type_params__[0]
",
        ),
        (
            "ordinary_header_captures_enclosing_private_parameter",
            true,
            r"class Outer[__T]:
    class Inner[U: (lambda: _Outer__T)]:
        pass
assert Outer.Inner.__type_params__[0].__bound__() is Outer.__type_params__[0]
",
        ),
        (
            "ordinary_header_preserves_private_capture_spellings",
            false,
            r"def outer(__T, _C__T):
    class C[U: (lambda: (__T, _C__T))]:
        pass
    return C
assert outer(7, 42).__type_params__[0].__bound__() == (7, 42)
",
        ),
        (
            "bound_generator",
            false,
            r"class C[__T: ((__T, _C__T) for k in [1])]:
    pass
a, b = next(C.__type_params__[0].__bound__)
assert a is b
",
        ),
        (
            "bound_generator_reverse",
            false,
            r"class C[__T: ((_C__T, __T) for k in [1])]:
    pass
a, b = next(C.__type_params__[0].__bound__)
assert a is b
",
        ),
        (
            "bound_alias_only",
            false,
            r"class C[__T: (_C__T for k in [1])]:
    pass
assert next(C.__type_params__[0].__bound__) is C.__type_params__[0]
",
        ),
        (
            "bound_direct",
            false,
            r"class C[__T: (__T, _C__T)]:
    pass
a, b = C.__type_params__[0].__constraints__
assert a is b
",
        ),
        (
            "bound_nested_lambda",
            false,
            r"class C[__T: ((lambda: (__T, _C__T))() for k in [1])]:
    pass
a, b = next(C.__type_params__[0].__bound__)
assert a is b
",
        ),
        (
            "earlier_parameter",
            false,
            r"class C[__T, U: ((__T, _C__T) for k in [1])]:
    pass
a, b = next(C.__type_params__[1].__bound__)
assert a is b is C.__type_params__[0]
",
        ),
        (
            "later_parameter",
            false,
            r"def f(__U, _C__U):
    class C[__T: ((__U, _C__U) for k in [1]), __U]:
        pass
    return C
C = f(42, 7)
a, b = next(C.__type_params__[0].__bound__)
assert a is b is C.__type_params__[1]
",
        ),
        (
            "unmangled_other_name",
            false,
            r"def f(__x, _C__x):
    class C[T: ((__x, _C__x) for k in [1])]:
        pass
    return C
C = f(7, 42)
assert next(C.__type_params__[0].__bound__) == (7, 42)
",
        ),
        (
            "generator_local_shadow",
            false,
            r"class C[__T: ((__T, _C__T) for _C__T in [3])]:
    pass
assert next(C.__type_params__[0].__bound__) == (3, 3)
",
        ),
        (
            "later_raw_capture",
            false,
            r"def outer(__U):
    class C[__T: (__U for _ in [1]), __U]:
        pass
    return C
C = outer(42)
g = C.__type_params__[0].__bound__
assert g.gi_code.co_freevars == ('__U',)
try:
    next(g)
except SystemError as error:
    assert str(error).startswith('no locals found')
else:
    raise AssertionError('expected SystemError')
",
        ),
        (
            "later_raw_direct",
            false,
            r"def outer(__U):
    class C[__T: __U, __U]:
        pass
    return C
C = outer(42)
try:
    C.__type_params__[0].__bound__
except SystemError as error:
    assert str(error).startswith('no locals found')
else:
    raise AssertionError('expected SystemError')
",
        ),
        (
            "later_raw_class_dict",
            false,
            r"class Outer:
    _C__U = 99
    class C[__T: __U, __U]:
        pass
try:
    Outer.C.__type_params__[0].__bound__
except SystemError as error:
    assert str(error).startswith('no locals found')
else:
    raise AssertionError('expected SystemError')
",
        ),
        (
            "later_lambda_parameter",
            false,
            r"class C[__T: (lambda __U: __U), __U]:
    pass
f = C.__type_params__[0].__bound__
assert f.__code__.co_varnames == ('__U',)
try:
    f(5)
except SystemError as error:
    assert str(error).startswith('no locals found')
else:
    raise AssertionError('expected SystemError')
",
        ),
        (
            "later_generator_target",
            false,
            r"class C[__T: (__U for __U in (5,)), __U]:
    pass
g = C.__type_params__[0].__bound__
assert g.gi_code.co_varnames == ('.0',)
try:
    next(g)
except SystemError as error:
    assert str(error).startswith('no locals found')
else:
    raise AssertionError('expected SystemError')
",
        ),
        (
            "later_target_and_capture",
            false,
            r"class C[__T: ((__U, _C__U) for __U in (5,)), __U]:
    pass
g = C.__type_params__[0].__bound__
assert g.gi_code.co_varnames == ('.0',)
assert next(g) == (5, 5)
",
        ),
        (
            "outer_class_dict_direct",
            false,
            r"class Outer:
    _C__T = 99
    class C[__T: (__T, _C__T)]:
        pass
assert Outer.C.__type_params__[0].__constraints__ == (99, 99)
",
        ),
        (
            "outer_class_dict_lambda",
            true,
            r"class Outer:
    _C__T = 99
    class C[__T: (lambda: (__T, _C__T))]:
        pass
p = Outer.C.__type_params__[0]
a, b = p.__bound__()
assert a is b is p
",
        ),
        (
            "outer_class_dict_base",
            false,
            r"seen = []
class B: pass
def base(a, b):
    seen.append((a, b))
    return B
class Outer:
    _C__T = 99
    class C[__T](base(__T, _C__T)):
        pass
a, b = seen[0]
assert a is b is Outer.C.__type_params__[0]
",
        ),
        (
            "base_alias",
            false,
            r"class B: pass
def base(a, b):
    assert a is b
    return B
class C[__T](base(__T, _C__T)):
    pass
",
        ),
        (
            "default_generator",
            true,
            r"class C[__T = ((__T, _C__T) for k in [1])]:
    pass
a, b = next(C.__type_params__[0].__default__)
assert a is b
",
        ),
        (
            "bound_inlined_comprehension",
            false,
            r"class C[__T: [(__T, _C__T) for k in [1]]]:
    pass
a, b = C.__type_params__[0].__bound__[0]
assert a is b
",
        ),
        (
            "bound_inlined_shadow",
            false,
            r"class C[__T: [(__T, _C__T) for _C__T in [3]]]:
    pass
assert C.__type_params__[0].__bound__ == [(3, 3)]
",
        ),
        (
            "base_generator",
            false,
            r"class B: pass
def base(g):
    a, b = next(g)
    assert a is b
    return B
class C[__T](base(((__T, _C__T) for k in [1]))):
    pass
",
        ),
        (
            "base_lambda",
            false,
            r"class B: pass
def base(f):
    a, b = f()
    assert a is b
    return B
class C[__T](base(lambda: (__T, _C__T))):
    pass
",
        ),
    ];
    for (version, executable) in targets() {
        for (name, requires_python_313, source) in cases {
            if requires_python_313 && version == PythonVersion::Py312 {
                continue;
            }
            let code = assert_matches_cpython(version, &executable, name, source);
            let mut child = Command::new(&executable)
                .args([
                    "-I",
                    "-S",
                    "-c",
                    "import marshal, sys; exec(marshal.loads(sys.stdin.buffer.read()))",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("start the pinned CPython runtime");
            child.stdin.take().unwrap().write_all(&code).unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{version:?} {name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}
