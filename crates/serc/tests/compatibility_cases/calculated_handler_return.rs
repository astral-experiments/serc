use crate::common::assert_matches_cpython_marshal;

#[test]
fn calculated_named_handler_return_one_manager() {
    assert_matches_cpython_marshal(
        r"from __future__ import annotations

def run_probe(snapshot: object, seed: int) -> dict[str, object]:
    script = textwrap.dedent(
        'sample payload'
    )
    with tempfile.TemporaryFile(prefix='sample-output-') as output_file:
        output_fd = output_file.fileno()
        payload = json.dumps(
            {
                'output_fd': output_fd,
                'seed': seed,
                'snapshot': snapshot,
            },
            sort_keys=True,
            separators=(',', ':'),
        )
        group_id: int | None = None
        try:
            process = launch(
                [EXECUTABLE, '-c', script],
                text=True,
                stdin=PIPE,
                stdout=PIPE,
                stderr=PIPE,
                env=dict(ENVIRONMENT),
                pass_fds=(output_fd,),
                start_new_session=HAS_GROUPS,
            )
            if HAS_GROUPS:
                group_id = process.pid
            stdout, stderr = communicate(
                process,
                input=payload,
                timeout=20,
            )
            completed = make_result(
                [EXECUTABLE, '-c', script],
                process.returncode,
                stdout=stdout,
                stderr=stderr,
            )
        except DeadlineError as exc:
            if group_id is not None:
                stop_group(group_id)
            return wrap_result(
                {
                    'exception': 'TimeoutExpired',
                    'message_digest': summarize(str(exc)),
                    'ok': False,
                    'stderr_digest': summarize(exc.stderr or ''),
                    'stdout_digest': summarize(exc.stdout or ''),
                }
            )
        except Exception as exc:
            return wrap_result(
                {
                    'exception': type(exc).__name__,
                    'failure_origin': 'caller',
                    'message_digest': summarize(str(exc)),
                    'ok': False,
                }
            )
        finally:
            if group_id is not None:
                stop_group(group_id)
        output_stat = os.fstat(output_fd)
        if output_stat.st_size == 0:
            return wrap_result(
                {
                    'exception': 'MissingOutput',
                    'message_digest': summarize('empty output'),
                    'ok': False,
                    'returncode': completed.returncode,
                    'stderr_digest': summarize(completed.stderr),
                    'stdout_digest': summarize(completed.stdout),
                }
            )
        if output_stat.st_size > 16_384:
            return wrap_result(
                {
                    'exception': 'LargeOutput',
                    'message_digest': summarize(str(output_stat.st_size)),
                    'ok': False,
                    'returncode': completed.returncode,
                }
            )
        try:
            os.lseek(output_fd, 0, os.SEEK_SET)
            raw_bytes = os.read(output_fd, 16_385)
        except OSError as exc:
            return wrap_result(
                {
                    'exception': type(exc).__name__,
                    'message_digest': summarize(str(exc)),
                    'ok': False,
                    'returncode': completed.returncode,
                }
            )
        if len(raw_bytes) > 16_384:
            return wrap_result(
                {
                    'exception': 'LargeOutput',
                    'message_digest': summarize(str(len(raw_bytes))),
                    'ok': False,
                    'returncode': completed.returncode,
                }
            )
        raw_output = raw_bytes.decode('utf-8', errors='replace')
    try:
        parsed = json.loads(raw_output)
    except json.JSONDecodeError as exc:
        return wrap_result(
            {
                'exception': 'JSONDecodeError',
                'message_digest': summarize(str(exc)),
                'ok': False,
                'returncode': completed.returncode,
            }
        )
    if isinstance(parsed, dict):
        parsed = dict(parsed)
        parsed['returncode'] = completed.returncode
        return wrap_result(parsed)
    return wrap_result({'ok': False, 'returncode': completed.returncode})
",
        "calculated_named_handler_return_one_manager.py",
    );
}
