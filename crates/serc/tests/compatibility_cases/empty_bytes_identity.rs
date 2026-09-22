use crate::common;

#[test]
fn matches_cpython_empty_bytes_from_constant_folding() {
    for expression in [
        "b'x' * 0",
        "0 * b'x'",
        "b'' * 3",
        "b'x'[:0]",
        "b'' + b''",
        "(b'x' * 0) * 2",
        "(b'x' * 0,)[0]",
    ] {
        for source in [
            format!("value = {expression}\n"),
            format!("def f():\n    return {expression}\n"),
        ] {
            common::assert_matches_cpython_marshal(&source, "empty_bytes_folding.py");
        }
    }
}

#[test]
fn matches_cpython_empty_bytes_cache_admission_order() {
    for source in [
        "a = b''; value = b'x' * 0\n",
        "value = b'x' * 0; a = b''\n",
        "def f(): return b'x' * 0\nvalue = b''\n",
        "value = b''\ndef f(): return b'x' * 0\n",
        "def f(): pass\nvalue = b'x' * 0\n",
        "value = b'x' * 0\ndef f(): pass\n",
        "def f(): return b'x' * 0\ndef g(): return b''\n",
        "def f():\n    def g(): return b'x' * 0\n    return b''\n",
        "value = (b'', b'x' * 0)\n",
        "value = (b'x' * 0, b'')\n",
        "value = b'x'[:0]; other = b'x' * 0\n",
        "value = b'x' * 0; other = b'x'[:0]\n",
        "def f():\n    try: unknown\n    except: pass\nvalue = b'x' * 0\n",
        "value = b'x' * 0\ntry: unknown\nexcept: pass\n",
        "value = b'x' * 0; other = data[b'':]\n",
        "other = data[:b'']; value = b'x' * 0\n",
        "def f(): return b'x' * 0\nother = data[::b'']\n",
        "match (b'x' * 0,):\n    case value: pass\n",
        "match (b'x' * 0,):\n    case (value,): pass\n",
        "match (b'x' * 0, b''):\n    case value: pass\n",
        "match (b'', b'x' * 0):\n    case value: pass\n",
    ] {
        common::assert_matches_cpython_marshal(source, "empty_bytes_cache_order.py");
    }
}

#[test]
fn matches_cpython_empty_bytes_cache_admissions_from_discarded_code() {
    for source in [
        "b'x' * 0\n",
        "b''; value = b'x' * 0\n",
        "if False: a = b''\nvalue = b'x' * 0\n",
        "if False:\n    def f(): pass\nvalue = b'x' * 0\n",
        "if False:\n    def f(): return b'x' * 0\nvalue = b''\n",
        "def f():\n    return\n    value = b'x' * 0\n",
        "def f():\n    return b'x' * 0\n    value = b''\n",
    ] {
        common::assert_matches_cpython_marshal(source, "empty_bytes_discarded.py");
    }
}
