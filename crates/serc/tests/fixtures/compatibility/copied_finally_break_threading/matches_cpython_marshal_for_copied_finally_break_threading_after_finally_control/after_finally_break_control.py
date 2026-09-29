def run(source_manager, temporary_manager, path_for, is_empty, upload, retry, finish, abort):
    completed = False
    try:
        with source_manager as source:
            part_number = 1
            while True:
                part_path = None
                stop = False
                try:
                    with temporary_manager as temporary:
                        part_path = path_for(temporary)
                        remaining = 3
                        while remaining:
                            chunk = source.read(remaining)
                            if not chunk:
                                break
                            temporary.write(chunk)
                            remaining -= len(chunk)
                    stop = is_empty(part_path)
                    if not stop:
                        result = upload(part_path, part_number)
                        if result != 0:
                            if retry(result):
                                return "RETRY"
                            raise RuntimeError(result)
                finally:
                    if part_path is not None:
                        part_path.unlink(missing_ok=True)
                if stop:
                    break
                part_number += 1
        finish()
        completed = True
        return "CREATED"
    finally:
        if not completed:
            abort()
