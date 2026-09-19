use crate::common;

#[test]
fn sync_named_handler_break_no_fallthrough() {
    common::assert_matches_cpython_marshal(
        r"def run(client):
    while True:
        try:
            raise Error
        except Error as error:
            if should_stop(error):
                break
            raise
        finally:
            if client is not None:
                client.close()
",
        "sync_named_handler_break_no_fallthrough.py",
    );
}
