use crate::common;

#[test]
fn matches_cpython_single_bare_raise_exit_copies() {
    for handler in ["Error", "Error as error", ""] {
        for recovery in [
            "try:\n    recover()\nexcept OtherError:\n    pass\n",
            "with manager():\n    recover()\n",
        ] {
            for finalizer in ["", "    finally:\n        finish()\n"] {
                let source = format!(
                    "def f(value):\n    try:\n        work()\n    except {handler}:\n{}        raise\n{finalizer}",
                    indent(recovery, 2)
                );
                common::assert_matches_cpython_marshal(&source, "single_bare_raise_exit_copies.py");
            }
        }
    }
}

#[test]
fn preserves_original_size_of_bare_raise_exits() {
    for passes in [4, 5, 9] {
        let source = format!(
            "def f(value):\n    try:\n        work(value)\n    except Error as error:\n        try:\n            recover(value)\n        except OtherError:\n            pass\n        {}raise\n    finally:\n        finish(value)\n",
            "pass; ".repeat(passes)
        );
        common::assert_matches_cpython_marshal(&source, "original_bare_raise_exit_size.py");
    }
}

fn indent(source: &str, depth: usize) -> String {
    let prefix = "    ".repeat(depth);
    let mut indented = String::new();
    for line in source.lines() {
        indented.push_str(&prefix);
        indented.push_str(line);
        indented.push('\n');
    }
    indented
}
