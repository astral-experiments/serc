def request(options, key, payload, label):
    address = options.base.rstrip('/') + '/action'
    body = encode(payload).encode('utf-8')
    allowed = {408, 409, 429, 500, 502, 503, 504}
    for attempt in range(1, options.max_attempts + 1):
        first_path = ''
        second_path = ''
        try:
            with files.open_named(delete=False) as first_handle:
                first_handle.write(body)
                first_path = first_handle.name
            with files.open_named('w', encoding='utf-8', delete=False) as second_handle:
                second_handle.write(f'address = {render(address)}\n')
                second_handle.write(f'key = {render(key)}\n')
                second_handle.write(f'input = {render(first_path)}\n')
                second_handle.write(f'limit = {options.timeout}\n')
                second_handle.write(f'connect = {min(10.0, options.timeout)}\n')
                second_path = second_handle.name
            completed = process.run(
                ['worker', '--config', second_path],
                check=False,
                capture_output=True,
                timeout=options.timeout + 5,
            )
            if completed.stdout:
                response_body, sep, status_bytes = completed.stdout.rpartition(b'\n')
            else:
                response_body, sep, status_bytes = b'', b'', b''
            status = int(status_bytes.decode('utf-8')) if sep and status_bytes.isdigit() else 0
            detail = response_body.decode('utf-8', errors='replace')
            if completed.returncode == 0 and 200 <= status < 300:
                return parse(decode(detail))
            retryable = completed.returncode != 0 or status in allowed
            if not retryable or attempt == options.max_attempts:
                stderr = completed.stderr.decode('utf-8', errors='replace')
                raise RuntimeError(f'{label}: {attempt} {status} {stderr} {detail}')
        except process.Timeout as error:
            if attempt == options.max_attempts:
                raise RuntimeError(f'{label}: {attempt} {options.timeout}') from error
        finally:
            prepare_cleanup()
            if first_path:
                Resource(first_path).close(missing_ok=True)
            if second_path:
                Resource(second_path).close(missing_ok=True)
        pause(min(2**attempt, 20))
    raise RuntimeError(f'{label}: exhausted')
