use crate::common::assert_executes;

#[test]
fn annotation_format_argument_does_not_shadow_user_values() {
    assert_executes(
        r#"
format = int
value: format

def f(x: format):
    pass

class C:
    format = str
    value: format
    def f(x: format):
        pass

def outer(format):
    def f(x: format):
        pass
    return f

assert __annotate__(1)["value"] is int
assert f.__annotations__["x"] is int
assert C.__annotations__["value"] is str
assert C.f.__annotations__["x"] is str
assert outer(float).__annotations__["x"] is float
for annotate in (__annotate__, f.__annotate__, C.__annotate__, C.f.__annotate__, outer(float).__annotate__):
    try:
        annotate(3)
    except NotImplementedError:
        pass
    else:
        raise AssertionError("annotation format guard was bypassed")
print("ok")
"#,
        "ok\n",
    );
}
