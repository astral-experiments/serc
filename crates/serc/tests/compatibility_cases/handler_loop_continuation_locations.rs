use crate::common;

#[test]
fn matches_cpython_handler_cleanup_before_separately_lowered_loop_condition() {
    for function in ["def", "async def"] {
        for loop_header in ["for item in items:", "while active:"] {
            for name in ["", " as exc"] {
                for leading in ["", "            n += 1\n"] {
                    for condition in ["flag", "n > 0", "True", "True or flag"] {
                        for following in [
                            "        if flag:\n            follow()\n",
                            "        if flag:\n            break\n",
                            "        if flag:\n            follow()\n        else:\n            other()\n",
                        ] {
                            let source = format!(
                                "{function} f(items, active, flag, n):\n    {loop_header}\n        try:\n            call()\n        except Exception{name}:\n{leading}            if {condition}:\n                done()\n{following}    after()\n"
                            );
                            common::assert_matches_cpython_marshal(
                                &source,
                                "handler_loop_continuation_locations.py",
                            );
                        }
                    }
                }
            }
        }
    }
}
