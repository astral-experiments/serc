use crate::common;

#[test]
fn taken_constant_conditions_preserve_empty_borrow_entries() {
    for header in ["def", "async def"] {
        for condition in ["not True", "not __debug__"] {
            for prefix in ["", "    obj.start()\n"] {
                for otherwise in ["", "    else:\n        obj.other()\n"] {
                    let source = format!(
                        "{header} f(obj):\n{prefix}    if {condition}:\n        obj.skip()\n{otherwise}    obj.run()\n"
                    );
                    common::assert_matches_cpython_marshal(&source, "taken_constant_borrow.py");
                }
            }
        }
    }
}
