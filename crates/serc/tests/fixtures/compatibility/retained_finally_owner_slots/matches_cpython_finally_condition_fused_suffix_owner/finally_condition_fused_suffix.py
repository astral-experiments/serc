def f(lock, filesystem, runner, temporary, destination):
    with lock:
        if not filesystem.exists(destination):
            if filesystem.exists(temporary):
                filesystem.remove(temporary)
            try:
                runner.run([temporary, destination], check=True, capture_output=True)
                filesystem.replace(temporary, destination)
            finally:
                if filesystem.exists(temporary):
                    filesystem.remove(temporary)
    return destination
