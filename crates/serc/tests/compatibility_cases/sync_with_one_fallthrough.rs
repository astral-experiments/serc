use crate::common;

#[test]
fn sync_with_one_fallthrough_chained_return_target() {
    let source = r"def fetch(retries, client_factory, method, endpoint, stats):
    tags = []
    for attempt in retries:
        with attempt:
            with client_factory() as client:
                request = client.build_request(method, endpoint)
                response = client.send(request)
                tags.extend([f'status:{response.status_code}', 'pool:false'])
                stats.timing(endpoint, tags=tags)
                if response.status_code == 200:
                    return response.json()
                elif method.upper() == 'GET' and response.status_code == 404:
                    return None
                else:
                    response.raise_for_status()
    return None
";
    common::assert_matches_cpython_marshal(
        source,
        "sync_with_one_fallthrough_chained_return_target.py",
    );
}

#[test]
fn sync_with_one_fallthrough_continue_target() {
    let source = r"def read_records(reader, seen_lock, seen, stats, output):
    with reader() as records:
        for line_number, record in enumerate(records):
            raw_key = record.get('key')
            if raw_key is not None:
                if isinstance(raw_key, (list, tuple)):
                    key = tuple(raw_key)
                else:
                    key = (str(raw_key),)
                with seen_lock:
                    if key in seen:
                        stats.increment('duplicate')
                        continue
                    else:
                        seen.add(key)
            record['line'] = line_number
            output.put(record)
            stats.increment('read')
";
    common::assert_matches_cpython_marshal(source, "sync_with_one_fallthrough_continue_target.py");
}

#[test]
fn sync_with_one_fallthrough_nested_return_target() {
    let source = r"def check_device(direct, bridge, credentials, cluster, search, logger):
    if direct:
        return False
    else:
        with bridge(
            credentials=credentials, cluster=cluster
        ) as connection:
            output, status = connection.exec_command('show model')
            match = search('component=(.+)', output)
            if not match:
                match = search('model=(.+)', output)
            if match:
                value = match.group(1).strip()
                if value == 'SUPPORTED':
                    logger.debug('Supported model')
                    return True
                else:
                    logger.error('Unknown model: %s', value)
                    return False
            else:
                logger.error('Model missing: %s', output)
        logger.error('Unable to confirm model')
        return False
";
    common::assert_matches_cpython_marshal(
        source,
        "sync_with_one_fallthrough_nested_return_target.py",
    );
}

#[test]
fn sync_with_one_fallthrough_direct_return_control() {
    let source = r"def direct_return(manager, condition, value, report):
    with manager:
        if condition:
            return value
        else:
            report(value)
    return None
";
    common::assert_matches_cpython_marshal(
        source,
        "sync_with_one_fallthrough_direct_return_control.py",
    );
}

#[test]
fn sync_with_one_fallthrough_two_arms_control() {
    let source = r"def two_fallthroughs(manager, condition, value, report):
    with manager:
        if condition:
            report(value)
        else:
            report(None)
    return None
";
    common::assert_matches_cpython_marshal(source, "sync_with_one_fallthrough_two_arms_control.py");
}

#[test]
fn matches_cpython_original_manager_break_donor_joins() {
    for (filename, source) in [
        (
            "manager_break_nested_assignment_donor_target.py",
            r"def run(values, outer, inner, state):
    for value in values:
        with outer(value), inner(value):
            if not value:
                break
            else:
                state.result = value
    return state
",
        ),
        (
            "manager_continue_nested_assignment_donor_control.py",
            r"def run(values, outer, inner, state):
    for value in values:
        with outer(value), inner(value):
            if not value:
                continue
            else:
                state.result = value
    return state
",
        ),
        (
            "manager_break_chained_call_donor_target.py",
            r"def send(values, manager):
    for value in values:
        with manager(value):
            if value.ready:
                return value.result
            elif value.retry:
                retry(value)
            else:
                report(value)
                break
    return None
",
        ),
        (
            "manager_return_chained_call_donor_control.py",
            r"def send(values, manager):
    for value in values:
        with manager(value):
            if value.ready:
                return value.result
            elif value.retry:
                retry(value)
            else:
                report(value)
                return None
    return None
",
        ),
    ] {
        common::assert_matches_cpython_marshal(source, filename);
    }
}
