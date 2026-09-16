use crate::common;

#[test]
fn matches_cpython_marshal_for_located_manager_nested_handler_join() {
    let source = r"def ensure_support(connection, errors, report):
    with connection.open() as resource:
        try:
            resource.execute('probe')
            resource.fetchone()
            return
        except errors.Failure:
            try:
                resource.execute('prepare')
                resource.execute('probe')
                resource.fetchone()
                return
            except errors.Failure as error:
                report(f'Feature unavailable: {error}')
";
    common::assert_matches_cpython_marshal(source, "located_manager_nested_handler_join.py");
}

#[test]
fn matches_cpython_marshal_for_located_manager_direct_handler_join() {
    let source = r"def ensure_support(connection, errors, report):
    with connection.open() as resource:
        try:
            resource.execute('probe')
            resource.fetchone()
            return
        except errors.Failure as error:
            report(f'Feature unavailable: {error}')
";
    common::assert_matches_cpython_marshal(source, "located_manager_direct_handler_join.py");
}
