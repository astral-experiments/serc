def f(pods, prefixes):
    try:
        return [pod for pod in pods if any(pod.startswith(prefix) for prefix in prefixes)]
    except Error:
        recover()
