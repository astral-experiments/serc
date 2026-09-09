use crate::common;

#[test]
fn matches_cpython_enclosing_cleanup_handler_join_locations() {
    for (function, suspension) in [
        ("def", ""),
        ("async def", ""),
        ("def", "    yield flag\n"),
        ("async def", "    yield flag\n"),
    ] {
        for name in ["", " as error"] {
            for condition in ["flag", "flag or other", "True"] {
                for body in ["work()", "value = other"] {
                    for (enclosing, cleanup) in [
                        ("try:", "    finally:\n        finish()\n"),
                        ("with manager:", ""),
                    ] {
                        let source = format!(
                            "{function} f(flag, other, manager):\n{suspension}    {enclosing}\n        try:\n            work()\n        except Exception{name}:\n            if {condition}:\n                {body}\n{cleanup}"
                        );
                        common::assert_matches_cpython_marshal(
                            &source,
                            "enclosing_cleanup_handler_join_locations.py",
                        );
                    }
                }
            }
        }
    }
}
