use crate::common;

#[test]
fn executes_surrogate_template_format_specs() {
    let source = r#"class EchoSpec:
    def __format__(self, spec):
        return spec


value = 7
width = 4
echo = EchoSpec()
templates = (
    t"{value:\ud800}",
    t"{value:\ufffd\ud800{width}\udfff�}",
    tr"{value:\ud800�}",
    rt"{value:\ud800�{width}\udfff}",
    tr"{value:\ud800�}" t"{value:\U0000dfff}",
    t"{value:\ud800{width}}" tr"{value:\udfff�{width}}",
    t"{value:{echo:\ud800�}}",
    tr"{value:{echo:\ud800�}}",
)
for template in templates:
    print(ascii(tuple(item.format_spec for item in template.interpolations)))
"#;

    common::assert_executes(
        source,
        r"('\ud800',)
('\ufffd\ud8004\udfff\ufffd',)
('\\ud800\ufffd',)
('\\ud800\ufffd4\\udfff',)
('\\ud800\ufffd', '\udfff')
('\ud8004', '\\udfff\ufffd4')
('\ud800\ufffd',)
('\\ud800\ufffd',)
",
    );
}

const INTERPOLATION_SOURCE_TEXT: &str = r#"class Echo:
    def __format__(self, spec):
        return spec

a, b, c = 1, 2, 3
formatted = (
    f"{a != b=:}",
    f"{a!=b=:}",
    f"{a != b != c=:}",
    f"{a != b and b != c=:}",
    f"{a if b != c else a=:}",
    f"{(a != b)=:}",
    f"{a != b=!s:}",
    f"{Echo():{a != b=:}}",
)
templates = (
    t"{a != b != c:}",
    t"{a != b != c=:}",
    t"{a != b and b != c:}",
    t"{a if b != c else a:}",
    t"{(a != b):}",
    t"{a != b!s:}",
    t"{a != b=}",
    t"""{(a # : fake ! conversion
):}""",
    t"""{(a # : fake ! conversion
)!s}""",
    t"""{(a # ! fake : conversion
)!r:>4}""",
    t"""{(a # ! fake : conversion
)}""",
)
"#;

#[test]
fn executes_interpolation_source_text() {
    let source = format!(
        "{INTERPOLATION_SOURCE_TEXT}\n\
         for value in formatted:\n    print(repr(value))\n\
         for value in templates:\n    print(repr(value.strings), repr(value.interpolations[0].expression))\n"
    );
    common::assert_executes(
        &source,
        "'a True'\n\
         'aTrue'\n\
         'a != b True'\n\
         'a != b and b True'\n\
         'a if b 1'\n\
         '(a != b)=True'\n\
         'a != b=True'\n\
         'a True'\n\
         ('', '') 'a != b'\n\
         ('a != b ', '') 'a != b'\n\
         ('', '') 'a != b and b'\n\
         ('', '') 'a if b'\n\
         ('', '') '(a != b)'\n\
         ('', '') 'a != b'\n\
         ('a != b=', '') 'a != b'\n\
         ('', '') '(a \\n)'\n\
         ('', '') '(a \\n)'\n\
         ('', '') '(a \\n)'\n\
         ('', '') '(a \\n)'\n",
    );
}

#[test]
fn large_template_string_tuple_matches_cpython_stack_folding_limit() {
    use std::fmt::Write as _;

    for count in [0, 1, 28, 29, 30, 31, 40] {
        for kind in ["adjacent", "distinct", "parts", "debug", "formatted"] {
            let mut source = String::from("def work(value):\n    return (t\"");
            for index in 0..count {
                match kind {
                    "adjacent" => source.push_str("{value}"),
                    "distinct" => write!(source, "word{index}{{value}}").unwrap(),
                    "parts" => source.push_str("word{value}\"\n        t\""),
                    "debug" => source.push_str("{value=}"),
                    "formatted" => source.push_str("{value:>3}"),
                    _ => unreachable!(),
                }
            }
            if kind == "distinct" {
                source.push_str("last");
            }
            source.push_str("\")\n");
            common::assert_matches_cpython_marshal(
                &source,
                &format!("template_string_tuple_{kind}_{count}.py"),
            );
        }
    }
}

#[test]
fn executes_large_template_strings_in_order() {
    use std::fmt::Write as _;

    let mut source = String::from(
        "calls = []\ndef record(value):\n    calls.append(value)\n    return value\nresult = t\"",
    );
    for index in 0..31 {
        write!(source, "part{index}{{record({index})}}").unwrap();
    }
    source.push_str(
        "end\"\nprint(len(result.strings), len(result.interpolations))\nprint(result.strings[0], result.strings[-1])\nprint(calls == list(range(31)))\nprint([value.value for value in result.interpolations] == calls)\n",
    );
    common::assert_executes(&source, "32 31\npart0 end\nTrue\nTrue\n");
}
